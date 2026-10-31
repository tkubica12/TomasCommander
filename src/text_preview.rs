use crate::files::FileResult;
use std::{ops::Range, path::Path};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Language {
    Plain,
    Python,
    Rust,
    Json,
    Env,
    Config,
    Script,
    Markup,
    Markdown,
}

impl Language {
    pub fn for_path(path: &Path) -> Self {
        let name = path
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("")
            .to_ascii_lowercase();
        if name == ".env" || name.starts_with(".env.") {
            return Self::Env;
        }
        match path
            .extension()
            .and_then(|ext| ext.to_str())
            .unwrap_or("")
            .to_ascii_lowercase()
            .as_str()
        {
            "py" => Self::Python,
            "rs" => Self::Rust,
            "json" => Self::Json,
            "env" => Self::Env,
            "toml" | "ini" | "yaml" | "yml" => Self::Config,
            "xml" | "html" | "htm" => Self::Markup,
            "md" | "markdown" => Self::Markdown,
            "ps1" | "sh" | "js" | "ts" | "css" => Self::Script,
            _ => Self::Plain,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Plain => "Text",
            Self::Python => "Python",
            Self::Rust => "Rust",
            Self::Json => "JSON",
            Self::Env => "Environment",
            Self::Config => "Configuration",
            Self::Script => "Source",
            Self::Markup => "Markup",
            Self::Markdown => "Markdown",
        }
    }
}

pub struct Code {
    pub source: String,
    pub language: Language,
    pub formatted: Option<String>,
    pub diagnostic: Option<String>,
}

impl Code {
    pub fn new(path: &Path, source: &str) -> FileResult<Self> {
        let language = Language::for_path(path);
        if source.len() > crate::viewer::MAX_TEXT_BYTES || source.lines().count() > 20_000 {
            return Err("Text exceeds the preview size/line limit.".into());
        }
        let (formatted, diagnostic) = if language == Language::Json {
            match serde_json::from_str::<serde_json::Value>(source) {
                Ok(value) => {
                    let text = serde_json::to_string_pretty(&value)
                        .map_err(|e| format!("Format JSON: {e}"))?;
                    match crate::viewer::validate_text(&text) {
                        Ok(()) => (Some(text), None),
                        Err(error) => (None, Some(format!("Autoformat unavailable: {error}"))),
                    }
                }
                Err(error) => (None, Some(format!("Invalid JSON: {error}"))),
            }
        } else {
            crate::viewer::validate_text(source)?;
            (None, None)
        };
        Ok(Self {
            source: source.to_owned(),
            language,
            formatted,
            diagnostic,
        })
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Token {
    Plain,
    Comment,
    String,
    Number,
    Keyword,
    Key,
    Function,
    Punctuation,
}

#[derive(Debug)]
pub struct Span {
    pub range: Range<usize>,
    pub token: Token,
}

// A bounded lexical preview, not a parser, formatter or executable language service.
pub fn spans(text: &str, language: Language) -> Vec<Span> {
    let mut spans: Vec<Span> = Vec::new();
    let mut pos = 0;
    let mut line_start = true;
    while pos < text.len() {
        let start = pos;
        let rest = &text[pos..];
        let ch = rest.chars().next().expect("nonempty text");
        let hash_comments = matches!(
            language,
            Language::Python | Language::Env | Language::Config | Language::Script
        );
        let slash_comments = matches!(language, Language::Rust | Language::Script);
        let token = if (hash_comments && ch == '#') || (slash_comments && rest.starts_with("//")) {
            pos += rest.find('\n').unwrap_or(rest.len());
            Token::Comment
        } else if (slash_comments && rest.starts_with("/*"))
            || (language == Language::Markup && rest.starts_with("<!--"))
        {
            let (prefix, close) = if language == Language::Markup {
                (4, "-->")
            } else {
                (2, "*/")
            };
            pos += rest[prefix..]
                .find(close)
                .map_or(rest.len(), |end| prefix + end + close.len());
            Token::Comment
        } else if ch == '"' || ch == '\'' || (language == Language::Script && ch == '`') {
            let triple = language == Language::Python
                && (rest.starts_with("\"\"\"") || rest.starts_with("'''"));
            let delimiter = if triple {
                &rest[..3]
            } else {
                &rest[..ch.len_utf8()]
            };
            pos += delimiter.len();
            while pos < text.len() {
                if text[pos..].starts_with('\\') {
                    pos += 1;
                    if let Some(next) = text[pos..].chars().next() {
                        pos += next.len_utf8();
                    }
                } else if text[pos..].starts_with(delimiter) {
                    pos += delimiter.len();
                    break;
                } else {
                    let next = text[pos..].chars().next().expect("nonempty string");
                    if !triple && next == '\n' && ch != '`' {
                        break;
                    }
                    pos += next.len_utf8();
                }
            }
            if language == Language::Json && text[pos..].trim_start().starts_with(':') {
                Token::Key
            } else {
                Token::String
            }
        } else if ch.is_ascii_digit()
            || (ch == '-' && rest.as_bytes().get(1).is_some_and(u8::is_ascii_digit))
        {
            pos += ch.len_utf8();
            while let Some(next) = text[pos..].chars().next() {
                if !next.is_ascii_alphanumeric() && !matches!(next, '.' | '_' | '+' | '-') {
                    break;
                }
                pos += next.len_utf8();
            }
            Token::Number
        } else if ch.is_alphabetic() || matches!(ch, '_' | '$') {
            pos += ch.len_utf8();
            while let Some(next) = text[pos..].chars().next() {
                if !next.is_alphanumeric() && !matches!(next, '_' | '$' | '-') {
                    break;
                }
                pos += next.len_utf8();
            }
            let word = &text[start..pos];
            let tail = text[pos..].trim_start();
            if keyword(word, language) {
                Token::Keyword
            } else if ((language == Language::Env || (language == Language::Config && line_start))
                && tail.starts_with('='))
                || (matches!(language, Language::Config | Language::Json) && tail.starts_with(':'))
            {
                Token::Key
            } else if tail.starts_with('(') {
                Token::Function
            } else if language == Language::Env {
                Token::String
            } else {
                Token::Plain
            }
        } else if ch.is_whitespace() {
            pos += ch.len_utf8();
            while let Some(next) = text[pos..].chars().next() {
                if !next.is_whitespace() {
                    break;
                }
                pos += next.len_utf8();
            }
            Token::Plain
        } else {
            pos += ch.len_utf8();
            Token::Punctuation
        };
        for ch in text[start..pos].chars() {
            if ch == '\n' {
                line_start = true;
            } else if !ch.is_whitespace() {
                line_start = false;
            }
        }
        if let Some(previous) = spans.last_mut().filter(|span| span.token == token) {
            previous.range.end = pos;
        } else {
            spans.push(Span {
                range: start..pos,
                token,
            });
        }
    }
    spans
}

fn keyword(word: &str, language: Language) -> bool {
    match language {
        Language::Python => matches!(
            word,
            "False"
                | "None"
                | "True"
                | "and"
                | "as"
                | "assert"
                | "async"
                | "await"
                | "break"
                | "class"
                | "continue"
                | "def"
                | "del"
                | "elif"
                | "else"
                | "except"
                | "finally"
                | "for"
                | "from"
                | "global"
                | "if"
                | "import"
                | "in"
                | "is"
                | "lambda"
                | "nonlocal"
                | "not"
                | "or"
                | "pass"
                | "raise"
                | "return"
                | "try"
                | "while"
                | "with"
                | "yield"
        ),
        Language::Rust => matches!(
            word,
            "as" | "async"
                | "await"
                | "break"
                | "const"
                | "continue"
                | "crate"
                | "dyn"
                | "else"
                | "enum"
                | "extern"
                | "false"
                | "fn"
                | "for"
                | "if"
                | "impl"
                | "in"
                | "let"
                | "loop"
                | "match"
                | "mod"
                | "move"
                | "mut"
                | "pub"
                | "ref"
                | "return"
                | "self"
                | "Self"
                | "static"
                | "struct"
                | "super"
                | "trait"
                | "true"
                | "type"
                | "unsafe"
                | "use"
                | "where"
                | "while"
        ),
        Language::Json | Language::Config => matches!(word, "true" | "false" | "null"),
        Language::Env => word == "export",
        Language::Script => matches!(
            word,
            "const"
                | "let"
                | "var"
                | "function"
                | "return"
                | "if"
                | "else"
                | "for"
                | "while"
                | "class"
                | "new"
                | "import"
                | "from"
                | "export"
                | "async"
                | "await"
                | "true"
                | "false"
                | "null"
                | "undefined"
                | "try"
                | "catch"
                | "throw"
                | "def"
                | "param"
        ),
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn language_detection_handles_dotenv_variants_and_case() {
        for name in [".env", ".env.local", ".ENV.production", "sample.env"] {
            assert_eq!(Language::for_path(Path::new(name)), Language::Env);
        }
        assert_eq!(Language::for_path(Path::new("script.PY")), Language::Python);
    }

    #[test]
    fn lexer_preserves_every_utf8_byte_and_distinguishes_python_strings_comments_and_keywords() {
        let text = "def hello(name):\n    text = \"ČEZ # not a comment\"\n    block = '''a\n# still string\nb'''\n    return 42 # comment\n";
        let spans = spans(text, Language::Python);
        assert_eq!(
            spans
                .iter()
                .map(|span| &text[span.range.clone()])
                .collect::<String>(),
            text
        );
        assert!(
            spans
                .iter()
                .any(|span| span.token == Token::Keyword && &text[span.range.clone()] == "def")
        );
        assert!(spans.iter().any(|span| span.token == Token::String
            && text[span.range.clone()].contains("# still string")));
        assert!(
            spans.iter().any(
                |span| span.token == Token::Comment && &text[span.range.clone()] == "# comment"
            )
        );
        for text in [
            "",
            "č",
            "\"unterminated\\",
            "/* unclosed",
            "\"a\\\"b\": true",
            "'\nnext",
            "# comment\nA=1",
        ] {
            let parts = spans_for_test(text);
            assert_eq!(parts, text);
        }
    }

    fn spans_for_test(text: &str) -> String {
        spans(text, Language::Script)
            .iter()
            .map(|span| &text[span.range.clone()])
            .collect()
    }

    #[test]
    fn json_formatting_never_replaces_source_and_invalid_input_stays_readable() {
        let source = "{\"číslo\":42,\"enabled\":true}";
        let code = Code::new(Path::new("sample.json"), source).unwrap();
        assert_eq!(code.source, source);
        assert!(code.formatted.as_ref().unwrap().contains('\n'));
        let bad = Code::new(Path::new("bad.json"), "{bad").unwrap();
        assert_eq!(bad.source, "{bad");
        assert!(bad.formatted.is_none());
        assert!(bad.diagnostic.unwrap().starts_with("Invalid JSON:"));
        let env = "# comment\nexport API_TOKEN=\"synthetic\"\nPORT=123\n";
        let parts = spans(env, Language::Env);
        assert!(
            parts
                .iter()
                .any(|span| span.token == Token::Key && &env[span.range.clone()] == "API_TOKEN")
        );
    }
}
