//! Read-only XLSX/XLSM cell-value extraction for the AI file harness.
//!
//! Only workbook/relationship/shared-string/worksheet XML parts are read.
//! Macros, external links, embedded objects and formulas are never evaluated;
//! formulas contribute their cached value only.

use std::{
    borrow::Cow,
    collections::BTreeMap,
    io::{Read, Seek},
};

pub(crate) const MAX_PART_BYTES: usize = 4 * 1024 * 1024;
const MAX_ENTRIES: usize = 10_000;
const MAX_SHEETS: usize = 1_000;
const MAX_SHARED_STRINGS: usize = 500_000;
/// Column XFD, the last column Excel allows.
const MAX_COLUMN: usize = 16_384;
const TOO_LARGE: &str = "Extracted spreadsheet text exceeds 32 MiB.";

type Result<T> = std::result::Result<T, String>;

pub(crate) const OLE: &str = "Encrypted/protected or legacy binary Office file (OLE container, not an Open XML package); content is not readable.";

/// Password-protected Open XML files and mislabeled legacy files are OLE compound
/// files, not ZIP packages; report that explicitly instead of a ZIP error.
pub(crate) fn ole_container<R: Read + Seek>(reader: &mut R) -> bool {
    let mut magic = [0u8; 8];
    let found = reader.read_exact(&mut magic).is_ok()
        && magic == [0xd0, 0xcf, 0x11, 0xe0, 0xa1, 0xb1, 0x1a, 0xe1];
    reader.seek(std::io::SeekFrom::Start(0)).is_ok() && found
}

pub(crate) fn text<R: Read + Seek>(
    mut reader: R,
    max_text: usize,
    stopped: &dyn Fn() -> bool,
) -> Result<String> {
    if ole_container(&mut reader) {
        return Err(OLE.into());
    }
    let mut archive =
        zip::ZipArchive::new(reader).map_err(|e| format!("Read spreadsheet package: {e}"))?;
    if archive.len() > MAX_ENTRIES {
        return Err("Spreadsheet package exceeds entry limit.".into());
    }
    let workbook = part(&mut archive, "xl/workbook.xml")?
        .ok_or("Spreadsheet package has no xl/workbook.xml.")?;
    let relationships = part(&mut archive, "xl/_rels/workbook.xml.rels")?
        .ok_or("Spreadsheet package has no workbook relationships.")?;
    let shared = match part(&mut archive, "xl/sharedStrings.xml")? {
        Some(xml) => shared_strings(&xml, stopped)?,
        None => Vec::new(),
    };
    let targets = worksheet_targets(&relationships)?;
    let sheets = sheets(&workbook)?;
    if sheets.is_empty() {
        return Err("Spreadsheet workbook lists no sheets.".into());
    }
    let mut text = String::new();
    for sheet in sheets {
        if stopped() {
            return Err("Cancelled during spreadsheet extraction.".into());
        }
        if !text.is_empty() {
            text.push('\n');
        }
        text.push_str(&format!(
            "[sheet {}{}]\n",
            clean(&sheet.name),
            if sheet.hidden { " (hidden)" } else { "" }
        ));
        let Some(target) = targets.get(&sheet.relationship) else {
            // Chart, dialog and macro sheets have no worksheet cell values.
            text.push_str("(not a worksheet; no cell values read)\n");
            continue;
        };
        match part(&mut archive, target) {
            Ok(Some(xml)) => worksheet(&xml, &shared, &mut text, max_text, stopped)?,
            Ok(None) => text.push_str("(worksheet part is missing)\n"),
            Err(error) if error.contains("exceeds 4 MiB") => {
                text.push_str("(worksheet XML exceeds 4 MiB; sheet skipped)\n")
            }
            Err(error) => return Err(error),
        }
        if text.len() > max_text {
            return Err(TOO_LARGE.into());
        }
    }
    Ok(text)
}

fn part<R: Read + Seek>(archive: &mut zip::ZipArchive<R>, name: &str) -> Result<Option<String>> {
    let mut member = match archive.by_name(name) {
        Ok(member) => member,
        Err(zip::result::ZipError::FileNotFound) => return Ok(None),
        Err(e) => return Err(format!("Read spreadsheet member: {e}")),
    };
    if member.size() > MAX_PART_BYTES as u64 {
        return Err(format!("Spreadsheet XML part {name} exceeds 4 MiB."));
    }
    let mut bytes = Vec::new();
    (&mut member)
        .take(MAX_PART_BYTES as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| format!("Extract spreadsheet XML: {e}"))?;
    if bytes.len() > MAX_PART_BYTES {
        return Err(format!("Spreadsheet XML part {name} exceeds 4 MiB."));
    }
    let bytes = bytes
        .strip_prefix(&[0xef, 0xbb, 0xbf])
        .unwrap_or(&bytes)
        .to_vec();
    let xml = String::from_utf8(bytes).map_err(|_| "Spreadsheet XML must be UTF-8.")?;
    if xml.contains("<!DOCTYPE") || xml.contains("<!ENTITY") {
        return Err("Spreadsheet XML entities are not supported.".into());
    }
    Ok(Some(xml))
}

struct Sheet {
    name: String,
    relationship: String,
    hidden: bool,
}

fn sheets(xml: &str) -> Result<Vec<Sheet>> {
    let mut sheets = Vec::new();
    for event in Tokens::new(xml) {
        if let Event::Start {
            name: "sheet",
            attrs,
            ..
        } = event?
        {
            if sheets.len() >= MAX_SHEETS {
                return Err("Spreadsheet has too many sheets.".into());
            }
            sheets.push(Sheet {
                name: attribute(attrs, "name").unwrap_or_default(),
                relationship: attribute(attrs, "id").unwrap_or_default(),
                hidden: attribute(attrs, "state").is_some_and(|state| state != "visible"),
            });
        }
    }
    Ok(sheets)
}

fn worksheet_targets(xml: &str) -> Result<BTreeMap<String, String>> {
    let mut targets = BTreeMap::new();
    for event in Tokens::new(xml) {
        if let Event::Start {
            name: "Relationship",
            attrs,
            ..
        } = event?
        {
            if attribute(attrs, "TargetMode").is_some_and(|mode| mode == "External")
                || !attribute(attrs, "Type").is_some_and(|kind| kind.ends_with("/worksheet"))
            {
                continue;
            }
            let (Some(id), Some(target)) = (attribute(attrs, "Id"), attribute(attrs, "Target"))
            else {
                continue;
            };
            let target = target.replace('\\', "/");
            let path = match target.strip_prefix('/') {
                Some(absolute) => absolute.to_owned(),
                None => format!("xl/{target}"),
            };
            if path.split('/').any(|part| part == ".." || part.is_empty()) {
                continue;
            }
            targets.insert(id, path);
        }
    }
    Ok(targets)
}

fn shared_strings(xml: &str, stopped: &dyn Fn() -> bool) -> Result<Vec<String>> {
    let mut strings = Vec::new();
    let mut current: Option<String> = None;
    let mut capture = false;
    let mut phonetic = 0usize;
    let mut total = 0usize;
    for (index, event) in Tokens::new(xml).enumerate() {
        if index % 4096 == 0 && stopped() {
            return Err("Cancelled during spreadsheet extraction.".into());
        }
        match event? {
            Event::Start { name: "si", .. } if strings.len() >= MAX_SHARED_STRINGS => {
                return Err("Spreadsheet has too many shared strings.".into());
            }
            Event::Start { name, empty, .. } => match name {
                "si" if empty => strings.push(String::new()),
                "si" => current = Some(String::new()),
                "rPh" if !empty => phonetic += 1,
                "t" if !empty && phonetic == 0 => capture = current.is_some(),
                _ => (),
            },
            Event::End { name } => match name {
                "si" => strings.extend(current.take()),
                "rPh" => phonetic = phonetic.saturating_sub(1),
                "t" => capture = false,
                _ => (),
            },
            Event::Text(text) => {
                if capture && let Some(current) = current.as_mut() {
                    total += text.len();
                    if total > MAX_PART_BYTES {
                        return Err("Spreadsheet shared strings exceed 4 MiB.".into());
                    }
                    current.push_str(&text);
                }
            }
        }
    }
    Ok(strings)
}

#[derive(Default)]
struct Cell {
    column: usize,
    kind: String,
    value: String,
    inline: String,
}

/// Cell values of one row; shared strings are borrowed, never copied per reference.
type Row<'s> = Vec<(usize, Cow<'s, str>)>;

fn cell_value(cell: Cell, shared: &[String]) -> Cow<'_, str> {
    match cell.kind.as_str() {
        "s" => Cow::Borrowed(
            cell.value
                .trim()
                .parse::<usize>()
                .ok()
                .and_then(|index| shared.get(index))
                .map_or("", String::as_str),
        ),
        "inlineStr" => Cow::Owned(cell.inline),
        "b" => match cell.value.trim() {
            "1" => Cow::Borrowed("TRUE"),
            "0" => Cow::Borrowed("FALSE"),
            other => Cow::Owned(other.to_owned()),
        },
        _ => Cow::Owned(cell.value),
    }
}

fn flush<'s>(row: &mut Option<Row<'s>>, rows: &mut Vec<Row<'s>>) {
    if let Some(mut cells) = row.take()
        && !cells.is_empty()
    {
        cells.sort_by_key(|(column, _)| *column);
        rows.push(cells);
    }
}

fn worksheet<'s>(
    xml: &str,
    shared: &'s [String],
    text: &mut String,
    max_text: usize,
    stopped: &dyn Fn() -> bool,
) -> Result<()> {
    let mut rows: Vec<Row<'s>> = Vec::new();
    let mut row: Option<Row<'s>> = None;
    let mut cell: Option<Cell> = None;
    let mut next_column = 0usize;
    let mut in_data = false;
    let mut capture_value = false;
    let mut capture_inline = false;
    let mut in_inline = false;
    let mut phonetic = 0usize;
    // Output bytes so far plus every buffered value and its separator, checked
    // before each cell is kept so one row cannot grow past the text budget.
    let mut size = text.len();
    for (index, event) in Tokens::new(xml).enumerate() {
        if index % 4096 == 0 && stopped() {
            return Err("Cancelled during spreadsheet extraction.".into());
        }
        match event? {
            Event::Start { name, attrs, empty } => match name {
                "sheetData" => in_data = !empty,
                "row" if in_data => {
                    // A row a malformed part never closed still ends here.
                    flush(&mut row, &mut rows);
                    next_column = 0;
                    if !empty {
                        row = Some(Vec::new());
                    }
                }
                "c" if row.is_some() => {
                    let column = match attribute(attrs, "r") {
                        Some(reference)
                            if reference.starts_with(|c: char| c.is_ascii_alphabetic()) =>
                        {
                            column_index(&reference)
                        }
                        _ => Some(next_column),
                    }
                    .filter(|column| *column < MAX_COLUMN);
                    // Cells beyond column XFD are invalid and skipped, never padded.
                    next_column = column.map_or(MAX_COLUMN, |column| column + 1);
                    if !empty && let Some(column) = column {
                        cell = Some(Cell {
                            column,
                            kind: attribute(attrs, "t").unwrap_or_default(),
                            ..Cell::default()
                        });
                    }
                }
                "v" if cell.is_some() && !empty => capture_value = true,
                "is" if cell.is_some() && !empty => in_inline = true,
                "rPh" if !empty => phonetic += 1,
                "t" if in_inline && phonetic == 0 && !empty => capture_inline = true,
                _ => (),
            },
            Event::End { name } => match name {
                "sheetData" => {
                    in_data = false;
                    flush(&mut row, &mut rows);
                }
                "row" => flush(&mut row, &mut rows),
                "c" => {
                    if let Some(done) = cell.take()
                        && let Some(cells) = row.as_mut()
                    {
                        let column = done.column;
                        let value = cell_value(done, shared);
                        if !value.chars().all(|c| c.is_whitespace() || c.is_control()) {
                            size += value.len() + 1;
                            if size > max_text {
                                return Err(TOO_LARGE.into());
                            }
                            cells.push((column, value));
                        }
                    }
                    capture_value = false;
                    in_inline = false;
                    capture_inline = false;
                }
                "v" => capture_value = false,
                "is" => in_inline = false,
                "rPh" => phonetic = phonetic.saturating_sub(1),
                "t" => capture_inline = false,
                _ => (),
            },
            Event::Text(fragment) => {
                if let Some(cell) = cell.as_mut() {
                    if capture_value {
                        cell.value.push_str(&fragment);
                    } else if capture_inline {
                        cell.inline.push_str(&fragment);
                    }
                }
            }
        }
    }
    flush(&mut row, &mut rows);
    let first = rows
        .iter()
        .filter_map(|cells| cells.first().map(|(column, _)| *column))
        .min()
        .unwrap_or(0);
    for cells in rows {
        let mut separators = 0;
        for (index, (column, value)) in cells.iter().enumerate() {
            // Tabs keep values aligned with their column relative to the leftmost used column.
            let tabs = (column - first).saturating_sub(separators);
            let space = usize::from(index > 0 && tabs == 0);
            // Padding is at most one tab per column up to XFD and is budgeted like values.
            if text.len() + space + tabs + value.len() + 1 > max_text {
                return Err(TOO_LARGE.into());
            }
            if space == 1 {
                text.push(' ');
            }
            text.extend(std::iter::repeat_n('\t', tabs));
            separators += tabs;
            text.extend(value.chars().map(|c| if c.is_control() { ' ' } else { c }));
        }
        text.push('\n');
    }
    Ok(())
}

fn column_index(reference: &str) -> Option<usize> {
    let mut column = 0usize;
    let mut letters = 0;
    for byte in reference.bytes() {
        if byte.is_ascii_alphabetic() {
            letters += 1;
            column = column * 26 + (byte.to_ascii_uppercase() - b'A' + 1) as usize;
            if letters > 3 {
                return None;
            }
        } else {
            break;
        }
    }
    (letters > 0 && column <= MAX_COLUMN).then(|| column - 1)
}

fn clean(value: &str) -> String {
    value
        .chars()
        .map(|c| if c.is_control() { ' ' } else { c })
        .collect()
}

enum Event<'a> {
    Start {
        name: &'a str,
        attrs: &'a str,
        empty: bool,
    },
    End {
        name: &'a str,
    },
    Text(Cow<'a, str>),
}

/// A minimal non-validating XML tokenizer for trusted-shape OOXML parts.
/// It never resolves external entities and refuses DTDs.
struct Tokens<'a> {
    xml: &'a str,
    position: usize,
}

impl<'a> Tokens<'a> {
    fn new(xml: &'a str) -> Self {
        Self { xml, position: 0 }
    }
}

fn local(name: &str) -> &str {
    name.rsplit(':').next().unwrap_or(name)
}

impl<'a> Iterator for Tokens<'a> {
    type Item = Result<Event<'a>>;

    fn next(&mut self) -> Option<Self::Item> {
        loop {
            let rest = &self.xml[self.position..];
            if rest.is_empty() {
                return None;
            }
            if !rest.starts_with('<') {
                let end = rest.find('<').unwrap_or(rest.len());
                self.position += end;
                return Some(Ok(Event::Text(unescape(&rest[..end]))));
            }
            let skip = |marker: &str, from: usize| {
                rest[from..]
                    .find(marker)
                    .map(|end| from + end + marker.len())
                    .ok_or_else(|| "Malformed spreadsheet XML.".to_owned())
            };
            if rest.starts_with("<!--") {
                match skip("-->", 4) {
                    Ok(end) => self.position += end,
                    Err(error) => return Some(Err(error)),
                }
                continue;
            }
            if let Some(data) = rest.strip_prefix("<![CDATA[") {
                let Some(end) = data.find("]]>") else {
                    return Some(Err("Malformed spreadsheet XML.".into()));
                };
                self.position += 9 + end + 3;
                return Some(Ok(Event::Text(Cow::Borrowed(&data[..end]))));
            }
            if rest.starts_with("<?") {
                match skip("?>", 2) {
                    Ok(end) => self.position += end,
                    Err(error) => return Some(Err(error)),
                }
                continue;
            }
            if rest.starts_with("<!") {
                return Some(Err("Spreadsheet XML declarations are not supported.".into()));
            }
            let mut quote = None;
            let mut end = None;
            for (index, character) in rest.char_indices().skip(1) {
                match (quote, character) {
                    (Some(open), c) if c == open => quote = None,
                    (Some(_), _) => (),
                    (None, '"' | '\'') => quote = Some(character),
                    (None, '<') => return Some(Err("Malformed spreadsheet XML.".into())),
                    (None, '>') => {
                        end = Some(index);
                        break;
                    }
                    _ => (),
                }
            }
            let Some(end) = end else {
                return Some(Err("Malformed spreadsheet XML.".into()));
            };
            self.position += end + 1;
            let inner = &rest[1..end];
            if let Some(name) = inner.strip_prefix('/') {
                return Some(Ok(Event::End {
                    name: local(name.trim()),
                }));
            }
            let (inner, empty) = match inner.strip_suffix('/') {
                Some(inner) => (inner, true),
                None => (inner, false),
            };
            let split = inner
                .find(|c: char| c.is_ascii_whitespace())
                .unwrap_or(inner.len());
            return Some(Ok(Event::Start {
                name: local(&inner[..split]),
                attrs: &inner[split..],
                empty,
            }));
        }
    }
}

fn attribute(attrs: &str, wanted: &str) -> Option<String> {
    let mut rest = attrs;
    loop {
        rest = rest.trim_start();
        let equals = rest.find('=')?;
        let name = rest[..equals].trim();
        let after = rest[equals + 1..].trim_start();
        let quote = after.chars().next().filter(|c| matches!(c, '"' | '\''))?;
        let close = after[1..].find(quote)? + 1;
        let value = &after[1..close];
        rest = &after[close + 1..];
        let prefixed = name.contains(':');
        if !name.starts_with("xmlns") && (name == wanted || prefixed && local(name) == wanted) {
            return Some(unescape(value).into_owned());
        }
    }
}

fn unescape(text: &str) -> Cow<'_, str> {
    if !text.contains('&') {
        return Cow::Borrowed(text);
    }
    let mut output = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(start) = rest.find('&') {
        output.push_str(&rest[..start]);
        rest = &rest[start..];
        let Some(end) = rest.find(';').filter(|end| *end <= 12) else {
            output.push('&');
            rest = &rest[1..];
            continue;
        };
        let entity = &rest[1..end];
        let decoded = match entity {
            "lt" => Some('<'),
            "gt" => Some('>'),
            "amp" => Some('&'),
            "quot" => Some('"'),
            "apos" => Some('\''),
            _ => entity
                .strip_prefix("#x")
                .or_else(|| entity.strip_prefix("#X"))
                .and_then(|hex| u32::from_str_radix(hex, 16).ok())
                .or_else(|| entity.strip_prefix('#').and_then(|dec| dec.parse().ok()))
                .and_then(char::from_u32),
        };
        match decoded {
            Some(character) => {
                output.push(character);
                rest = &rest[end + 1..];
            }
            None => {
                output.push('&');
                rest = &rest[1..];
            }
        }
    }
    output.push_str(rest);
    Cow::Owned(output)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tokenizer_handles_namespaces_entities_cdata_and_refuses_declarations() {
        let events: Vec<_> = Tokens::new(
            r#"<?xml version="1.0"?><x:c r="B2" t='s'><!-- note --><x:v>&lt;1&#x41;&#66;&amp;</x:v><t><![CDATA[a<b]]></t><e/></x:c>"#,
        )
        .collect::<Result<_>>()
        .unwrap();
        assert!(matches!(
            events[0],
            Event::Start {
                name: "c",
                empty: false,
                ..
            }
        ));
        if let Event::Start { attrs, .. } = &events[0] {
            assert_eq!(attribute(attrs, "r").as_deref(), Some("B2"));
            assert_eq!(attribute(attrs, "t").as_deref(), Some("s"));
        }
        assert!(matches!(&events[2], Event::Text(text) if text == "<1AB&"));
        assert!(matches!(&events[5], Event::Text(text) if text == "a<b"));
        assert!(matches!(
            events[7],
            Event::Start {
                name: "e",
                empty: true,
                ..
            }
        ));
        assert!(Tokens::new("<!DOCTYPE x><a/>").any(|event| event.is_err()));
        assert!(Tokens::new("<a b='1'").any(|event| event.is_err()));
        assert_eq!(column_index("A1"), Some(0));
        assert_eq!(column_index("AB12"), Some(27));
        assert_eq!(column_index("XFD1"), Some(16_383));
        assert_eq!(column_index("XFE1"), None);
        assert_eq!(column_index("12"), None);
        assert_eq!(
            attribute(r#" xmlns:r="urn:r" name="Q&amp;A" r:id="rId3""#, "id").as_deref(),
            Some("rId3")
        );
    }

    fn package(shared: &str, sheet: &str) -> std::io::Cursor<Vec<u8>> {
        use std::io::Write;
        let mut writer = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
        let stored = zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Stored);
        for (name, xml) in [
            (
                "xl/workbook.xml",
                r#"<workbook xmlns:r="urn:r"><sheets><sheet name="S" r:id="rId1"/></sheets></workbook>"#,
            ),
            (
                "xl/_rels/workbook.xml.rels",
                r#"<Relationships><Relationship Id="rId1" Type="urn:x/worksheet" Target="worksheets/sheet1.xml"/></Relationships>"#,
            ),
            ("xl/sharedStrings.xml", shared),
            ("xl/worksheets/sheet1.xml", sheet),
        ] {
            writer.start_file(name, stored).unwrap();
            writer.write_all(xml.as_bytes()).unwrap();
        }
        let mut cursor = writer.finish().unwrap();
        cursor.set_position(0);
        cursor
    }

    fn timed(shared: &str, sheet: &str, max_text: usize) -> Result<String> {
        let started = std::time::Instant::now();
        let result = text(package(shared, sheet), max_text, &|| false);
        assert!(started.elapsed() < std::time::Duration::from_secs(2));
        result
    }

    #[test]
    fn repeated_huge_shared_strings_fail_at_the_cell_that_exceeds_the_budget() {
        let shared = format!("<sst><si><t>{}</t></si></sst>", "x".repeat(4_000_000));
        let cells = r#"<c t="s"><v>0</v></c>"#.repeat(150_000);
        for close in ["</row>", ""] {
            let sheet =
                format!("<worksheet><sheetData><row>{cells}{close}</sheetData></worksheet>");
            assert_eq!(timed(&shared, &sheet, 32 << 20).unwrap_err(), TOO_LARGE);
        }
        let many = format!("<sst>{}</sst>", "<si/>".repeat(MAX_SHARED_STRINGS + 1));
        assert!(
            timed(&many, "<worksheet/>", 32 << 20)
                .unwrap_err()
                .contains("too many shared strings")
        );
    }

    #[test]
    fn unclosed_rows_are_kept_and_far_columns_are_bounded() {
        let shared = "<sst><si><t>shared</t></si></sst>";
        let unclosed = r#"<worksheet><sheetData><row><c t="s"><v>0</v></c><row><c><v>2</v></c><c r="XFE1"><v>beyond</v></c></sheetData></worksheet>"#;
        assert_eq!(
            timed(shared, unclosed, 32 << 20).unwrap(),
            "[sheet S]\nshared\n2\n"
        );
        let after_last = r#"<worksheet><sheetData><row><c r="B1"><v>a</v></c><c r="XFD1"><v>b</v></c><c><v>c</v></c></row></sheetData></worksheet>"#;
        let text = timed(shared, after_last, 32 << 20).unwrap();
        assert_eq!(text.matches('\t').count(), 16_382);
        assert!(text.ends_with("\tb\n") && !text.contains('c'), "{text}");
        let mut far = String::from(r#"<worksheet><sheetData><row><c r="A1"><v>1</v></c></row>"#);
        for row in 2..200 {
            far.push_str(&format!(r#"<row><c r="XFD{row}"><v>1</v></c></row>"#));
        }
        far.push_str("</sheetData></worksheet>");
        assert_eq!(timed(shared, &far, 1 << 20).unwrap_err(), TOO_LARGE);
    }
}
