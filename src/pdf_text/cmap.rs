//! Bounded ToUnicode CMap parsing and decoding.
//!
//! lopdf's CMap parser expands every code of every `bfrange` (a 30-byte
//! `<00000000> <FFFFFFFF>` range means billions of entries) and decodes whole
//! strings into unbounded intermediates. ToUnicode streams are therefore
//! parsed here into non-overlapping code spans that are never expanded, and
//! text is decoded code by code against an output budget.

use std::collections::BTreeMap;

/// Mapping entries kept per CMap (bfchar entries, ranges and array items).
pub(super) const MAX_ENTRIES: usize = 131_072;
/// Longer targets (the format allows 256) are skipped so one code stays small.
pub(super) const MAX_TARGET_UNITS: usize = 16;
const MAX_CODESPACES: usize = 256;

pub(super) enum Decoded {
    Done,
    /// The decoder cannot decode this font's strings at all.
    Failed,
    /// Output would exceed the caller's budget.
    Full,
    Expired,
}

#[derive(Clone, Copy)]
struct Span {
    end: u32,
    /// First code of the original range; the last target unit counts from it.
    origin: u32,
    start: u32,
    /// Zero marks a code the CMap covers but cannot map (replacement).
    len: u8,
}

#[derive(Default)]
pub(super) struct CMap {
    codespaces: Vec<(u32, u32, u8)>,
    /// Non-overlapping spans per code length (1-4 bytes), keyed by first code.
    spans: [BTreeMap<u32, Span>; 4],
    units: Vec<u16>,
    pub(super) entries: usize,
}

/// Parses a ToUnicode CMap, keeping at most `budget` entries. `None` means the
/// stream has no usable mapping or parsing ran out of time.
pub(super) fn parse(data: &[u8], budget: usize, expired: &dyn Fn() -> bool) -> Option<CMap> {
    let budget = budget.min(MAX_ENTRIES);
    let mut lexer = Lexer { data, at: 0 };
    let mut cmap = CMap::default();
    let mut source = Vec::new();
    let mut other = Vec::new();
    let mut steps = 0usize;
    while let Some(token) = lexer.next() {
        steps += 1;
        if steps.is_multiple_of(65_536) && expired() {
            return None;
        }
        if cmap.entries >= budget {
            break;
        }
        let Token::Word(word) = token else {
            continue;
        };
        match word {
            b"begincodespacerange" => {
                while let Some(Token::Hex(low)) = lexer.next() {
                    let Some(Token::Hex(high)) = lexer.next() else {
                        break;
                    };
                    if let (Some(low), Some(high)) =
                        (code(low, &mut source), code(high, &mut other))
                        && low.1 == high.1
                        && cmap.codespaces.len() < MAX_CODESPACES
                    {
                        cmap.codespaces.push((low.0, high.0, low.1));
                    }
                }
            }
            b"beginbfchar" => {
                while let Some(Token::Hex(raw)) = lexer.next() {
                    let target = match lexer.next() {
                        Some(Token::Hex(target)) => target,
                        // Glyph-name targets are not Unicode; skip the pair.
                        Some(Token::Word(name)) if name.starts_with(b"/") => continue,
                        _ => break,
                    };
                    if cmap.entries >= budget {
                        break;
                    }
                    if let Some((value, len)) = code(raw, &mut source)
                        && hex(target, 2 * MAX_TARGET_UNITS, &mut other)
                    {
                        cmap.insert(len, value, value, &other);
                    }
                }
            }
            b"beginbfrange" => {
                while let Some(Token::Hex(low)) = lexer.next() {
                    let Some(Token::Hex(high)) = lexer.next() else {
                        break;
                    };
                    let range = match (code(low, &mut source), code(high, &mut other)) {
                        (Some(low), Some(high)) if low.1 == high.1 && low.0 <= high.0 => {
                            Some((low.0, high.0, low.1))
                        }
                        _ => None,
                    };
                    match lexer.next() {
                        Some(Token::Hex(target)) => {
                            if let Some((low, high, len)) = range
                                && cmap.entries < budget
                                && hex(target, 2 * MAX_TARGET_UNITS, &mut other)
                            {
                                cmap.insert(len, low, high, &other);
                            }
                        }
                        Some(Token::Open) => {
                            let mut next = range.map(|(low, _, _)| low);
                            loop {
                                match lexer.next() {
                                    Some(Token::Hex(target)) => {
                                        let (Some((_, high, len)), Some(current)) = (range, next)
                                        else {
                                            continue;
                                        };
                                        if current > high || cmap.entries >= budget {
                                            continue;
                                        }
                                        if !hex(target, 2 * MAX_TARGET_UNITS, &mut other) {
                                            other.clear();
                                        }
                                        cmap.insert(len, current, current, &other);
                                        next = current.checked_add(1);
                                    }
                                    Some(Token::Close) => break,
                                    _ => return cmap.finish(),
                                }
                            }
                            // Codes without an array item decode as replacements, as in lopdf.
                            if let (Some((_, high, len)), Some(current)) = (range, next)
                                && current <= high
                                && cmap.entries < budget
                            {
                                cmap.insert(len, current, high, &[]);
                            }
                        }
                        _ => break,
                    }
                }
            }
            _ => (),
        }
    }
    cmap.finish()
}

/// Reads a 1-4 byte source code.
fn code(raw: &[u8], scratch: &mut Vec<u8>) -> Option<(u32, u8)> {
    if !hex(raw, 4, scratch) {
        return None;
    }
    let value = scratch
        .iter()
        .fold(0u32, |value, byte| value << 8 | u32::from(*byte));
    Some((value, scratch.len() as u8))
}

/// Decodes a hex string body of at most `max` bytes into `out`.
fn hex(raw: &[u8], max: usize, out: &mut Vec<u8>) -> bool {
    out.clear();
    let mut high = None;
    for byte in raw {
        if is_space(*byte) {
            continue;
        }
        let Some(nibble) = (*byte as char).to_digit(16) else {
            return false;
        };
        match high.take() {
            None => high = Some(nibble as u8),
            Some(high) => {
                if out.len() == max {
                    return false;
                }
                out.push(high << 4 | nibble as u8);
            }
        }
    }
    if let Some(high) = high {
        if out.len() == max {
            return false;
        }
        out.push(high << 4);
    }
    !out.is_empty()
}

impl CMap {
    /// Maps `low..=high` to the UTF-16BE `target`; later definitions win overlaps.
    fn insert(&mut self, len: u8, low: u32, high: u32, target: &[u8]) {
        let start = self.units.len() as u32;
        self.units.extend(target.chunks(2).map(|pair| match *pair {
            [high, low] => u16::from_be_bytes([high, low]),
            [single] => u16::from(single),
            _ => 0xFFFD,
        }));
        self.entries += 1;
        let map = &mut self.spans[usize::from(len) - 1];
        if let Some((&first, &span)) = map.range(..low).next_back()
            && span.end >= low
        {
            map.insert(
                first,
                Span {
                    end: low - 1,
                    ..span
                },
            );
            if span.end > high {
                map.insert(high + 1, span);
            }
        }
        let covered: Vec<u32> = map.range(low..=high).map(|(first, _)| *first).collect();
        for first in covered {
            if let Some(span) = map.remove(&first)
                && span.end > high
            {
                map.insert(high + 1, span);
            }
        }
        map.insert(
            low,
            Span {
                end: high,
                origin: low,
                start,
                len: target.len().div_ceil(2) as u8,
            },
        );
    }

    fn finish(self) -> Option<Self> {
        self.spans
            .iter()
            .any(|spans| !spans.is_empty())
            .then_some(self)
    }

    fn in_codespace(&self, code: u32, len: u8) -> bool {
        // Codespace bounds apply to each byte independently (ISO 32000-1, 9.7.5.1).
        self.codespaces.iter().any(|&(low, high, size)| {
            size == len
                && (0..u32::from(len)).all(|index| {
                    let shift = 8 * index;
                    let byte = (code >> shift) as u8;
                    (low >> shift) as u8 <= byte && byte <= (high >> shift) as u8
                })
        })
    }

    fn emit(&self, code: u32, len: u8, sink: &mut Utf16) -> bool {
        let Some((_, span)) = self.spans[usize::from(len) - 1]
            .range(..=code)
            .next_back()
            .filter(|(_, span)| span.end >= code)
        else {
            return false;
        };
        let start = span.start as usize;
        let Some((last, leading)) = self.units[start..start + usize::from(span.len)].split_last()
        else {
            sink.replace();
            return true;
        };
        for unit in leading {
            sink.push(*unit);
        }
        sink.push(last.wrapping_add((code - span.origin) as u16));
        true
    }

    /// Splits `bytes` into codes as lopdf does (the shortest code inside a
    /// codespace or, without codespaces, with a mapping) and appends their
    /// text, stopping once `output` exceeds `limit` bytes.
    pub(super) fn decode(
        &self,
        bytes: &[u8],
        output: &mut String,
        limit: usize,
        expired: &dyn Fn() -> bool,
        replaced: &mut usize,
    ) -> Decoded {
        let mut sink = Utf16::new(output, replaced);
        let mut code = 0u32;
        let mut len = 0u8;
        for (index, byte) in bytes.iter().enumerate() {
            if index % 16_384 == 16_383 && expired() {
                return Decoded::Expired;
            }
            if len == 4 {
                if !self.emit(code, 4, &mut sink) {
                    sink.replace();
                }
                (code, len) = (0, 0);
            }
            len += 1;
            code = code << 8 | u32::from(*byte);
            let decoded = if self.codespaces.is_empty() {
                self.emit(code, len, &mut sink)
            } else if self.in_codespace(code, len) {
                if !self.emit(code, len, &mut sink) {
                    sink.replace();
                }
                true
            } else {
                false
            };
            if decoded {
                (code, len) = (0, 0);
                if sink.output.len() > limit {
                    return Decoded::Full;
                }
            }
        }
        if len > 0 && !self.emit(code, len, &mut sink) {
            sink.replace();
        }
        sink.finish();
        if sink.output.len() > limit {
            Decoded::Full
        } else {
            Decoded::Done
        }
    }
}

/// UTF-16 to UTF-8 with surrogate pairing across codes; replacements are
/// counted as undecodable instead of being written.
pub(super) struct Utf16<'o> {
    pub(super) output: &'o mut String,
    pending: Option<u16>,
    replaced: &'o mut usize,
}

impl<'o> Utf16<'o> {
    pub(super) fn new(output: &'o mut String, replaced: &'o mut usize) -> Self {
        Self {
            output,
            pending: None,
            replaced,
        }
    }

    pub(super) fn push(&mut self, unit: u16) {
        if let Some(high) = self.pending.take() {
            if (0xDC00..=0xDFFF).contains(&unit) {
                let value =
                    0x10000 + ((u32::from(high) - 0xD800) << 10) + (u32::from(unit) - 0xDC00);
                self.output.extend(char::from_u32(value));
                return;
            }
            *self.replaced += 1;
        }
        match unit {
            0xD800..=0xDBFF => self.pending = Some(unit),
            0xDC00..=0xDFFF | 0xFFFD => *self.replaced += 1,
            _ => self.output.extend(char::from_u32(u32::from(unit))),
        }
    }

    pub(super) fn replace(&mut self) {
        self.push(0xFFFD);
    }

    pub(super) fn finish(&mut self) {
        if self.pending.take().is_some() {
            *self.replaced += 1;
        }
    }
}

enum Token<'a> {
    Hex(&'a [u8]),
    Open,
    Close,
    Word(&'a [u8]),
    Other,
}

fn is_space(byte: u8) -> bool {
    matches!(byte, b'\0' | b'\t' | b'\n' | b'\x0c' | b'\r' | b' ')
}

fn is_delimiter(byte: u8) -> bool {
    matches!(
        byte,
        b'(' | b')' | b'<' | b'>' | b'[' | b']' | b'{' | b'}' | b'/' | b'%'
    )
}

/// A minimal PostScript tokenizer for CMap programs; nothing is executed.
struct Lexer<'a> {
    data: &'a [u8],
    at: usize,
}

impl<'a> Lexer<'a> {
    fn next(&mut self) -> Option<Token<'a>> {
        loop {
            let byte = *self.data.get(self.at)?;
            let rest = &self.data[self.at..];
            match byte {
                _ if is_space(byte) => self.at += 1,
                b'%' => {
                    self.at += rest
                        .iter()
                        .position(|byte| matches!(byte, b'\r' | b'\n'))
                        .unwrap_or(rest.len());
                }
                b'<' if rest.get(1) == Some(&b'<') => {
                    self.at += 2;
                    return Some(Token::Other);
                }
                b'<' => {
                    let end = rest.iter().position(|byte| *byte == b'>')?;
                    self.at += end + 1;
                    return Some(Token::Hex(&rest[1..end]));
                }
                b'[' => {
                    self.at += 1;
                    return Some(Token::Open);
                }
                b']' => {
                    self.at += 1;
                    return Some(Token::Close);
                }
                b'(' => {
                    let mut depth = 0usize;
                    let mut escaped = false;
                    let mut end = rest.len();
                    for (index, byte) in rest.iter().enumerate() {
                        match (escaped, byte) {
                            (true, _) => escaped = false,
                            (false, b'\\') => escaped = true,
                            (false, b'(') => depth += 1,
                            (false, b')') => {
                                depth -= 1;
                                if depth == 0 {
                                    end = index + 1;
                                    break;
                                }
                            }
                            _ => (),
                        }
                    }
                    self.at += end;
                    return Some(Token::Other);
                }
                _ if byte == b'/' || !is_delimiter(byte) => {
                    let length = 1 + rest[1..]
                        .iter()
                        .position(|byte| is_space(*byte) || is_delimiter(*byte))
                        .unwrap_or(rest.len() - 1);
                    self.at += length;
                    return Some(Token::Word(&rest[..length]));
                }
                _ => {
                    self.at += 1;
                    return Some(Token::Other);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Duration, Instant};

    fn decode(cmap: &CMap, bytes: &[u8]) -> (String, usize) {
        let mut output = String::new();
        let mut replaced = 0;
        assert!(matches!(
            cmap.decode(bytes, &mut output, usize::MAX, &|| false, &mut replaced),
            Decoded::Done
        ));
        (output, replaced)
    }

    #[test]
    fn ranges_are_never_expanded_and_later_entries_win() {
        let started = Instant::now();
        let full = parse(
            b"/CIDInit /ProcSet findresource begin\n1 begincodespacerange <00000000> <FFFFFFFF> endcodespacerange\n2 beginbfrange <00000000> <FFFFFFFF> <0041> <000000> <FFFFFF> <0061> endbfrange\nendcmap",
            MAX_ENTRIES,
            &|| false,
        )
        .unwrap();
        assert_eq!(full.entries, 2);
        assert_eq!(
            decode(&full, &[0, 0, 0, 0, 0, 0, 0, 1, 0, 0, 0, 25]).0,
            "ABZ"
        );
        assert!(started.elapsed() < Duration::from_secs(1));

        let cmap = parse(
            b"% comment <00>\n(literal [<01>]) 1 begincodespacerange\n<00> <FF>\nendcodespacerange\n2 beginbfrange\n<20> <7E> <0020>\n<41> <43> [<0078> <D835DC9C> <00660069>]\nendbfrange\n2 beginbfchar\n<42> <0062>\n<44> <00440044>\nendbfchar\n1 beginbfrange <50> <52> [<0070>] endbfrange",
            MAX_ENTRIES,
            &|| false,
        )
        .unwrap();
        let (text, replaced) = decode(&cmap, b" ABCDEPQR");
        assert_eq!(text, " xbfiDDEp");
        assert_eq!(replaced, 2);
        let mut surrogate =
            parse(b"1 beginbfchar <01> <D835DC9C> endbfchar", 10, &|| false).unwrap();
        assert_eq!(decode(&surrogate, &[1]).0, "\u{1D49C}");
        surrogate.codespaces.push((0, 0xFF, 1));
        assert_eq!(
            decode(&surrogate, &[1, 2, 1]),
            ("\u{1D49C}\u{1D49C}".into(), 1)
        );
    }

    #[test]
    fn oversized_targets_codes_and_entry_budgets_are_skipped() {
        let data = format!(
            "1 begincodespacerange <00> <FF> endcodespacerange 1 beginbfchar <01> <{}> endbfchar 1 beginbfchar <0102030405> <0042> endbfchar 2 beginbfchar <02> <{}> <03> <0043> endbfchar",
            "0041".repeat(MAX_TARGET_UNITS + 1),
            "0041".repeat(MAX_TARGET_UNITS)
        );
        let cmap = parse(data.as_bytes(), MAX_ENTRIES, &|| false).unwrap();
        assert_eq!(cmap.entries, 2);
        let (text, replaced) = decode(&cmap, &[1, 2, 3]);
        assert_eq!(text, format!("{}C", "A".repeat(MAX_TARGET_UNITS)));
        assert_eq!(replaced, 1);
        let mut many = String::from("1 beginbfchar ");
        for code in 0..300u32 {
            many.push_str(&format!("<{code:04X}> <0041> "));
        }
        many.push_str("endbfchar");
        assert_eq!(parse(many.as_bytes(), 100, &|| false).unwrap().entries, 100);
        assert!(parse(b"1 beginbfchar <01> /space endbfchar", 10, &|| false).is_none());
    }

    #[test]
    fn decoding_stops_at_the_output_budget_and_deadline() {
        let target = "0041".repeat(MAX_TARGET_UNITS);
        let cmap = parse(
            format!("1 beginbfchar <01> <{target}> endbfchar").as_bytes(),
            MAX_ENTRIES,
            &|| false,
        )
        .unwrap();
        let bytes = vec![1u8; 1 << 20];
        let mut output = String::new();
        let mut replaced = 0;
        assert!(matches!(
            cmap.decode(&bytes, &mut output, 4096, &|| false, &mut replaced),
            Decoded::Full
        ));
        assert!(output.len() <= 4096 + MAX_TARGET_UNITS);
        output.clear();
        assert!(matches!(
            cmap.decode(&bytes, &mut output, usize::MAX, &|| true, &mut replaced),
            Decoded::Expired
        ));
        assert!(output.len() <= 16_384 * MAX_TARGET_UNITS);
    }
}
