use crate::files::FileResult;
use scraper::{ElementRef, Html, Selector};

pub(crate) fn markdown(input: &str) -> FileResult<String> {
    if input.len() > 65_536 {
        return Err("Semantic HTML preview exceeds 64 KiB.".into());
    }
    let document = Html::parse_document(input);
    if document.tree.nodes().count() > 4000 {
        return Err("HTML exceeds 4000 DOM nodes.".into());
    }
    let mut output = String::new();
    render(document.root_element(), 0, &mut output)?;
    if output.len() > 65_536 {
        return Err("Converted HTML exceeds the native render limit.".into());
    }
    Ok(output)
}

fn text(input: &str, output: &mut String) {
    for character in input.chars() {
        if "\\`*_{}[]<>|".contains(character) {
            output.push('\\');
        }
        output.push(if character.is_whitespace() {
            ' '
        } else {
            character
        });
    }
}

fn children(element: ElementRef<'_>, depth: usize, output: &mut String) -> FileResult<()> {
    for child in element.children() {
        if let Some(element) = ElementRef::wrap(child) {
            render(element, depth + 1, output)?;
        } else if let Some(value) = child.value().as_text() {
            text(&value.text, output);
        }
    }
    Ok(())
}

fn render(element: ElementRef<'_>, depth: usize, output: &mut String) -> FileResult<()> {
    if depth > 32 || output.len() > 65_536 {
        return Err("HTML depth/output exceeds native preview limits.".into());
    }
    let name = element.value().name();
    match name {
        "head" | "script" | "style" | "template" | "noscript" | "link" | "meta" => (),
        "img" => output.push_str(" [image omitted] "),
        "iframe" | "object" | "embed" | "svg" | "canvas" | "math" | "input" | "button"
        | "select" | "textarea" => output.push_str(" [interactive/embedded content omitted] "),
        "br" => output.push_str("  \n"),
        "hr" => output.push_str("\n\n---\n\n"),
        "h1" | "h2" | "h3" | "h4" | "h5" | "h6" => {
            output.push_str("\n\n");
            output.push_str(&"#".repeat(usize::from(name.as_bytes()[1] - b'0')));
            output.push(' ');
            children(element, depth, output)?;
            output.push_str("\n\n");
        }
        "strong" | "b" | "em" | "i" | "del" | "s" => {
            let marker = match name {
                "strong" | "b" => "**",
                "em" | "i" => "*",
                _ => "~~",
            };
            output.push_str(marker);
            children(element, depth, output)?;
            output.push_str(marker);
        }
        "pre" | "code" => {
            let raw = element.text().collect::<String>();
            let length = raw.split(|c| c != '`').map(str::len).max().unwrap_or(0) + 1;
            let fence = "`".repeat(length.max(if name == "pre" { 3 } else { 1 }));
            if name == "pre" {
                output.push_str(&format!("\n\n{fence}text\n{raw}\n{fence}\n\n"));
            } else {
                output.push_str(&format!("{fence} {raw} {fence}"));
            }
        }
        "a" => {
            if let Some(href) = element.value().attr("href") {
                let mut destination = Vec::new();
                for byte in href.as_bytes() {
                    if byte.is_ascii_control() || matches!(*byte, b'<' | b'>' | b'\\' | b' ') {
                        destination.extend_from_slice(format!("%{byte:02X}").as_bytes());
                    } else {
                        destination.push(*byte);
                    }
                }
                let destination = String::from_utf8(destination)
                    .map_err(|error| format!("HTML link encoding: {error}"))?;
                output.push('[');
                children(element, depth, output)?;
                output.push_str(&format!("](<{destination}>)"));
            } else {
                children(element, depth, output)?;
            }
        }
        "li" => {
            if element.value().attr("value").is_some() {
                return Err("HTML list item value overrides are not supported.".into());
            }
            let parents: Vec<_> = element
                .ancestors()
                .filter_map(ElementRef::wrap)
                .filter(|parent| matches!(parent.value().name(), "ol" | "ul"))
                .collect();
            output.push('\n');
            output.push_str(&"    ".repeat(parents.len().saturating_sub(1)));
            if let Some(parent) = parents
                .first()
                .filter(|parent| parent.value().name() == "ol")
            {
                if parent.value().attr("reversed").is_some() {
                    return Err("Reversed HTML lists are not supported.".into());
                }
                let start = parent
                    .value()
                    .attr("start")
                    .unwrap_or("1")
                    .parse::<u32>()
                    .map_err(|_| "HTML ordered list start must be a nonnegative integer.")?;
                let previous = element
                    .prev_siblings()
                    .filter_map(ElementRef::wrap)
                    .filter(|sibling| sibling.value().name() == "li")
                    .count() as u32;
                let ordinal = start
                    .checked_add(previous)
                    .filter(|value| *value <= 999_999_999)
                    .ok_or("HTML ordered list numbering exceeds native Markdown limits.")?;
                output.push_str(&format!("{ordinal}. "));
            } else {
                output.push_str("- ");
            }
            children(element, depth, output)?;
            output.push('\n');
        }
        "blockquote" => {
            let mut quote = String::new();
            children(element, depth, &mut quote)?;
            output.push_str("\n\n");
            for line in quote.trim().lines() {
                output.push_str(&format!("> {line}\n"));
            }
            output.push('\n');
        }
        "table" => {
            if element
                .ancestors()
                .filter_map(ElementRef::wrap)
                .any(|parent| parent.id() != element.id() && parent.value().name() == "table")
            {
                return Err("Nested HTML tables are not supported.".into());
            }
            let rows = Selector::parse("tr").map_err(|error| format!("HTML rows: {error}"))?;
            let mut table = Vec::new();
            for row in element.select(&rows) {
                let mut cells = Vec::new();
                for cell in row
                    .child_elements()
                    .filter(|e| matches!(e.value().name(), "td" | "th"))
                {
                    if cell.value().attr("rowspan").is_some()
                        || cell.value().attr("colspan").is_some()
                    {
                        return Err(
                            "HTML table row/column spans are not supported by the native viewer."
                                .into(),
                        );
                    }
                    let mut content = String::new();
                    children(cell, depth + 1, &mut content)?;
                    cells.push(content.replace(['\r', '\n'], " "));
                }
                if !cells.is_empty() {
                    table.push(cells);
                }
            }
            let width = table.iter().map(Vec::len).max().unwrap_or(0);
            if width > 64 || table.len() > 1000 {
                return Err("HTML table exceeds native row/column limits.".into());
            }
            output.push_str("\n\n");
            for (index, row) in table.iter().enumerate() {
                output.push('|');
                for column in 0..width {
                    output.push_str(row.get(column).map_or("", String::as_str));
                    output.push('|');
                }
                output.push('\n');
                if index == 0 {
                    output.push_str(&format!("|{}\n", "---|".repeat(width)));
                }
            }
            output.push('\n');
        }
        _ => {
            let block = matches!(
                name,
                "p" | "div" | "section" | "article" | "ul" | "ol" | "blockquote"
            );
            if block {
                output.push_str("\n\n");
            }
            children(element, depth, output)?;
            if block {
                output.push_str("\n\n");
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn semantic_html_preserves_structures_and_never_activates_resources() {
        let result = markdown("<h1>Příliš &amp; text</h1><p><strong>bold</strong> <em>em</em></p><ul><li>one</li></ul><table><tr><th>A</th></tr><tr><td>42</td></tr></table><pre>let a = 1;</pre><a href='https://example.org/č'>link</a><script>SECRET_SCRIPT</script><img src='https://invalid.example/image'>").unwrap();
        assert!(result.contains("# Příliš & text"));
        assert!(result.contains("**bold**"));
        assert!(result.contains("|A|\n|---|\n|42|"));
        assert!(result.contains("https://example.org/č"));
        assert!(!result.contains("SECRET_SCRIPT"));
        assert!(!result.contains("invalid.example"));
        assert!(result.contains("[image omitted]"));
        assert!(markdown("<table><tr><td colspan='2'>x</td></tr></table>").is_err());
        assert!(markdown(&"<div>".repeat(40)).is_err());
    }

    #[test]
    fn lists_quotes_and_unsupported_tables_are_explicit() {
        let result = markdown("<div><section><ol start='3'><li>first<ul><li>nested</li></ul></li><li>second</li></ol><blockquote><p>quoted</p></blockquote></section></div>").unwrap();
        assert!(result.contains("\n3. first"));
        assert!(result.contains("\n4. second"));
        assert!(result.contains("\n    - nested"));
        assert!(result.contains("> quoted"));
        assert!(markdown("<ol reversed><li>x</li></ol>").is_err());
        assert!(markdown("<ol><li value='7'>x</li></ol>").is_err());
        assert!(
            markdown("<table><tr><td><table><tr><td>x</td></tr></table></td></tr></table>")
                .is_err()
        );
    }
}
