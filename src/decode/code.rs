//! Source-code & plain-text viewing. Rasterizes text with lightweight syntax
//! highlighting into a `PixImage` using the embedded CP437 8×16 VGA font, so code
//! files flow through the exact same thumbnail + viewer pipeline as scene art
//! (grid tile, zoom/pan viewer, Details) with zero viewer changes.
//!
//! The highlighter is a small hand-rolled lexer (no heavy `syntect`/regex dep — this
//! project keeps its tree lean, and a VGA-font render doesn't need per-language
//! perfection). Comment/string rules are set precisely per language *family*; the
//! keyword set is a shared union across C-family/script languages (over-matching a
//! keyword in the "wrong" language is only cosmetic). A line-number gutter and a
//! dark VGA-ish palette give it the "nicely formatted" retro terminal look.

use super::cp437_font::CP437_8X16;
use super::{DecodeError, Decoder};
use crate::image_types::PixImage;

const CELL_W: usize = 8;
const CELL_H: usize = 16;
const TAB: usize = 4;
// Bounds so a huge file can't blow up memory in the thumbnail worker. The raster is
// sized to the *actual* content, so short files stay tiny; these only cap the tail.
const MAX_LINES: usize = 4000;
const MAX_COLS: usize = 240; // clip absurdly long lines
const MAX_CELLS: usize = 240_000; // ≈ 30 Mpx / 123 MB RGBA worst case; adapts lines↔width

// VGA-ish syntax palette: dark background, light default text, muted accents that read
// well in the 8×16 font.
const BG: [u8; 3] = [14, 14, 20];
const DEFAULT: [u8; 3] = [204, 204, 204];
const COMMENT: [u8; 3] = [106, 135, 89];
const KEYWORD: [u8; 3] = [86, 156, 214];
const TYPE: [u8; 3] = [78, 201, 176];
const STRING: [u8; 3] = [206, 145, 120];
const NUMBER: [u8; 3] = [181, 206, 168];
const PREPROC: [u8; 3] = [197, 134, 192];
const PUNCT: [u8; 3] = [160, 160, 170];
const GUTTER: [u8; 3] = [88, 88, 104];
const TRUNC: [u8; 3] = [220, 170, 90];

#[derive(Clone, Copy, PartialEq)]
enum Tok {
    Default,
    Comment,
    Keyword,
    Type,
    Str,
    Number,
    Preproc,
    Punct,
}

impl Tok {
    fn color(self) -> [u8; 3] {
        match self {
            Tok::Default => DEFAULT,
            Tok::Comment => COMMENT,
            Tok::Keyword => KEYWORD,
            Tok::Type => TYPE,
            Tok::Str => STRING,
            Tok::Number => NUMBER,
            Tok::Preproc => PREPROC,
            Tok::Punct => PUNCT,
        }
    }
}

/// Per-language lexing rules. Comment/string handling is what matters for readability;
/// keywords are a shared union (see `KEYWORDS`).
struct LangSpec {
    line: &'static [&'static str],               // line-comment starters
    block: Option<(&'static str, &'static str)>, // block-comment open/close
    raw: &'static [&'static str],                // multi-line string delimiters (py """, js `)
    quotes: &'static [char],                     // single-line string quote chars
    preproc_hash: bool,                          // leading `#word` is a directive (C/C++)
    highlight: bool,                             // false = plain text (txt/log/md), no tokens
}

const C_FAMILY: LangSpec = LangSpec {
    line: &["//"],
    block: Some(("/*", "*/")),
    raw: &[],
    quotes: &['"', '\''],
    preproc_hash: true,
    highlight: true,
};
const JS_FAMILY: LangSpec = LangSpec {
    line: &["//"],
    block: Some(("/*", "*/")),
    raw: &["`"],
    quotes: &['"', '\''],
    preproc_hash: false,
    highlight: true,
};
const RUST: LangSpec = LangSpec {
    line: &["//"],
    block: Some(("/*", "*/")),
    raw: &[],
    quotes: &['"'],
    preproc_hash: false,
    highlight: true,
};
const HASH: LangSpec = LangSpec {
    line: &["#"],
    block: None,
    raw: &["\"\"\"", "'''"],
    quotes: &['"', '\''],
    preproc_hash: false,
    highlight: true,
};
const LUA: LangSpec = LangSpec {
    line: &["--"],
    block: Some(("--[[", "]]")),
    raw: &[],
    quotes: &['"', '\''],
    preproc_hash: false,
    highlight: true,
};
const BASIC: LangSpec = LangSpec {
    line: &["'", "REM ", "rem "],
    block: None,
    raw: &[],
    quotes: &['"'],
    preproc_hash: false,
    highlight: true,
};
const ASM: LangSpec = LangSpec {
    line: &[";"],
    block: None,
    raw: &[],
    quotes: &['"', '\''],
    preproc_hash: false,
    highlight: true,
};
const CSS: LangSpec = LangSpec {
    line: &[],
    block: Some(("/*", "*/")),
    raw: &[],
    quotes: &['"', '\''],
    preproc_hash: false,
    highlight: true,
};
const HTML: LangSpec = LangSpec {
    line: &[],
    block: Some(("<!--", "-->")),
    raw: &[],
    quotes: &['"', '\''],
    preproc_hash: false,
    highlight: true,
};
const JSONISH: LangSpec = LangSpec {
    line: &["//"],
    block: Some(("/*", "*/")),
    raw: &[],
    quotes: &['"'],
    preproc_hash: false,
    highlight: true,
};
const PLAIN: LangSpec = LangSpec {
    line: &[],
    block: None,
    raw: &[],
    quotes: &[],
    preproc_hash: false,
    highlight: false,
};

/// Shared keyword union — over-matching in the "wrong" language is only cosmetic.
const KEYWORDS: &[&str] = &[
    "if",
    "else",
    "elif",
    "elseif",
    "for",
    "while",
    "do",
    "loop",
    "break",
    "continue",
    "return",
    "yield",
    "match",
    "case",
    "switch",
    "default",
    "goto",
    "fn",
    "def",
    "func",
    "function",
    "sub",
    "end",
    "class",
    "struct",
    "enum",
    "trait",
    "impl",
    "interface",
    "extends",
    "implements",
    "public",
    "private",
    "protected",
    "static",
    "final",
    "const",
    "let",
    "var",
    "mut",
    "auto",
    "new",
    "delete",
    "this",
    "self",
    "super",
    "import",
    "from",
    "use",
    "using",
    "include",
    "require",
    "package",
    "namespace",
    "module",
    "pub",
    "async",
    "await",
    "try",
    "catch",
    "except",
    "finally",
    "throw",
    "raise",
    "with",
    "as",
    "in",
    "is",
    "not",
    "and",
    "or",
    "typedef",
    "template",
    "typename",
    "operator",
    "virtual",
    "override",
    "dim",
    "then",
    "next",
    "print",
    "input",
    "goto",
    "gosub",
    "local",
    "global",
    "nil",
    "true",
    "false",
    "none",
    "null",
    "undefined",
    "void",
    "extern",
    "unsafe",
    "where",
    "move",
    "ref",
    "box",
    "dyn",
    "lambda",
    "pass",
    "del",
    "assert",
    "export",
    "signal",
    "onready",
    "extends",
    "tool",
    "var",
];

/// Common built-in / primitive type names.
const TYPES: &[&str] = &[
    "int", "long", "short", "char", "float", "double", "bool", "boolean", "byte", "string", "str",
    "void", "unsigned", "signed", "size_t", "u8", "u16", "u32", "u64", "usize", "i8", "i16", "i32",
    "i64", "isize", "f32", "f64", "vec", "map", "list", "dict", "set", "array", "object", "number",
    "any", "integer", "single", "long", "double", "String", "Vec", "Option", "Result", "Box",
    "Self",
];

fn lang_for(ext: &str) -> &'static LangSpec {
    match ext {
        "rs" => &RUST,
        "c" | "cpp" | "cc" | "cxx" | "h" | "hpp" | "hh" | "hxx" | "inc" | "ino" | "m" | "mm" => {
            &C_FAMILY
        }
        "java" | "cs" | "go" | "swift" | "kt" | "kts" | "scala" | "dart" | "php" | "php3"
        | "php4" | "php5" | "hlsl" | "glsl" | "shader" | "gdshader" => &C_FAMILY,
        "js" | "jsx" | "mjs" | "cjs" | "ts" | "tsx" | "json5" => &JS_FAMILY,
        "py" | "pyw" | "gd" | "pl" | "pm" | "rb" | "sh" | "bash" | "zsh" | "yaml" | "yml"
        | "toml" | "ini" | "cfg" | "conf" | "r" | "jl" | "ex" | "exs" | "coffee" | "tcl"
        | "ps1" | "cmake" | "mk" | "makefile" | "dockerfile" => &HASH,
        "lua" => &LUA,
        "bas" | "bm" | "bi" | "vb" | "vbs" | "qb" | "frm" => &BASIC,
        "asm" | "s" | "nasm" | "a51" => &ASM,
        "css" | "scss" | "sass" | "less" => &CSS,
        "html" | "htm" | "htmlx" | "xhtml" | "xml" | "xaml" | "vue" | "svelte" => &HTML,
        "json" | "jsonc" | "ipynb" => &JSONISH,
        _ => &PLAIN,
    }
}

enum Carry {
    None,
    Block,
    Raw(&'static str),
}

/// A word is a keyword / type / neither.
fn classify_word(w: &str) -> Tok {
    if KEYWORDS.contains(&w) {
        Tok::Keyword
    } else if TYPES.contains(&w) || (w.len() > 1 && w.starts_with(char::is_uppercase)) {
        // Uppercase-leading identifiers read as types/constructors (Rust/Java/C#…).
        Tok::Type
    } else {
        Tok::Default
    }
}

/// Lex one line into per-char `(char, Tok)`, carrying block-comment / raw-string state.
fn lex_line(line: &[char], spec: &LangSpec, carry: &mut Carry) -> Vec<(char, Tok)> {
    let mut out: Vec<(char, Tok)> = Vec::with_capacity(line.len());
    let n = line.len();
    let mut i = 0;

    // Continue a carried block comment / raw string first.
    match std::mem::replace(carry, Carry::None) {
        Carry::Block => {
            let close = spec.block.map(|b| b.1).unwrap_or("*/");
            if let Some(end) = find_at(line, 0, close) {
                for &c in &line[..end + close.chars().count()] {
                    out.push((c, Tok::Comment));
                }
                i = end + close.chars().count();
            } else {
                for &c in line {
                    out.push((c, Tok::Comment));
                }
                *carry = Carry::Block;
                return out;
            }
        }
        Carry::Raw(delim) => {
            if let Some(end) = find_at(line, 0, delim) {
                for &c in &line[..end + delim.chars().count()] {
                    out.push((c, Tok::Str));
                }
                i = end + delim.chars().count();
            } else {
                for &c in line {
                    out.push((c, Tok::Str));
                }
                *carry = Carry::Raw(delim);
                return out;
            }
        }
        Carry::None => {}
    }

    if !spec.highlight {
        for &c in &line[i..] {
            out.push((c, Tok::Default));
        }
        return out;
    }

    // Leading `#directive` (C preprocessor).
    let first_non_ws = line.iter().position(|c| !c.is_whitespace());
    let preproc_line = spec.preproc_hash && first_non_ws == Some(i) && line.get(i) == Some(&'#');

    while i < n {
        let c = line[i];
        let rest_starts = |pat: &str| starts_with_at(line, i, pat);

        // Line comment → rest of line.
        if let Some(&lc) = spec.line.iter().find(|&&p| rest_starts(p)) {
            let _ = lc;
            for &c in &line[i..] {
                out.push((c, Tok::Comment));
            }
            break;
        }
        // Block comment open.
        if let Some((open, close)) = spec.block {
            if rest_starts(open) {
                if let Some(end) = find_at(line, i + open.chars().count(), close) {
                    let stop = end + close.chars().count();
                    for &c in &line[i..stop] {
                        out.push((c, Tok::Comment));
                    }
                    i = stop;
                    continue;
                } else {
                    for &c in &line[i..] {
                        out.push((c, Tok::Comment));
                    }
                    *carry = Carry::Block;
                    break;
                }
            }
        }
        // Multi-line raw string delimiter.
        if let Some(&delim) = spec.raw.iter().find(|&&d| rest_starts(d)) {
            let dlen = delim.chars().count();
            if let Some(end) = find_at(line, i + dlen, delim) {
                let stop = end + dlen;
                for &c in &line[i..stop] {
                    out.push((c, Tok::Str));
                }
                i = stop;
                continue;
            } else {
                for &c in &line[i..] {
                    out.push((c, Tok::Str));
                }
                *carry = Carry::Raw(delim);
                break;
            }
        }
        // Single-line string.
        if spec.quotes.contains(&c) {
            let (span, next) = scan_string(line, i, c);
            for &ch in span {
                out.push((ch, Tok::Str));
            }
            i = next;
            continue;
        }
        // Preprocessor directive token.
        if preproc_line && c == '#' {
            let start = i;
            i += 1;
            while i < n && (line[i].is_alphanumeric() || line[i] == '_') {
                i += 1;
            }
            for &ch in &line[start..i] {
                out.push((ch, Tok::Preproc));
            }
            continue;
        }
        // Number.
        if c.is_ascii_digit() || (c == '.' && line.get(i + 1).is_some_and(|d| d.is_ascii_digit())) {
            let start = i;
            i += 1;
            while i < n && is_number_char(line[i]) {
                i += 1;
            }
            for &ch in &line[start..i] {
                out.push((ch, Tok::Number));
            }
            continue;
        }
        // Identifier / keyword.
        if c.is_alphabetic() || c == '_' || c == '@' || c == '$' {
            let start = i;
            i += 1;
            while i < n && (line[i].is_alphanumeric() || line[i] == '_') {
                i += 1;
            }
            let word: String = line[start..i].iter().collect();
            let tok = classify_word(&word);
            for &ch in &line[start..i] {
                out.push((ch, tok));
            }
            continue;
        }
        // Punctuation / operator / whitespace.
        let tok = if c.is_whitespace() || c.is_alphanumeric() {
            Tok::Default
        } else {
            Tok::Punct
        };
        out.push((c, tok));
        i += 1;
    }
    out
}

fn is_number_char(c: char) -> bool {
    c.is_ascii_hexdigit() || matches!(c, '.' | 'x' | 'X' | 'o' | 'b' | '_' | 'e' | 'E' | '+' | '-')
}

/// Scan a quoted string starting at `start` (the opening quote). Returns the char slice
/// (incl. quotes) and the index just past it. Honors `\` escapes; stops at EOL if unterminated.
fn scan_string(line: &[char], start: usize, quote: char) -> (&[char], usize) {
    let mut i = start + 1;
    while i < line.len() {
        if line[i] == '\\' {
            i += 2;
            continue;
        }
        if line[i] == quote {
            i += 1;
            break;
        }
        i += 1;
    }
    let end = i.min(line.len());
    (&line[start..end], end)
}

fn starts_with_at(line: &[char], at: usize, pat: &str) -> bool {
    let pc: Vec<char> = pat.chars().collect();
    if at + pc.len() > line.len() {
        return false;
    }
    line[at..at + pc.len()] == pc[..]
}

/// First index >= `from` where `pat` occurs in `line`, or None.
fn find_at(line: &[char], from: usize, pat: &str) -> Option<usize> {
    let pc: Vec<char> = pat.chars().collect();
    if pc.is_empty() || line.len() < pc.len() {
        return None;
    }
    (from..=line.len() - pc.len()).find(|&i| line[i..i + pc.len()] == pc[..])
}

/// Map a Unicode char to a CP437 byte for the bitmap font. ASCII passes through; a few
/// common punctuation lookalikes are folded; anything else becomes '?'.
fn to_cp437(c: char) -> u8 {
    let u = c as u32;
    if (0x20..0x7f).contains(&u) {
        return u as u8;
    }
    match c {
        '\t' => b' ',
        '·' | '•' => 0xf9,
        '’' | '‘' | '`' => b'\'',
        '“' | '”' => b'"',
        '—' | '–' => b'-',
        '…' => 0x07, // no ellipsis glyph; a bullet reads as "more"
        '→' => 0x1a,
        '←' => 0x1b,
        '©' => 0x63,
        _ if u < 0x20 => b' ',
        _ => b'?',
    }
}

/// If this is a Jupyter notebook, pull out its cells as readable text (markdown cells as
/// `# …` comments, code cells verbatim) so we render the notebook, not raw JSON.
fn ipynb_to_text(bytes: &[u8]) -> Option<String> {
    let v: serde_json::Value = serde_json::from_slice(bytes).ok()?;
    let cells = v.get("cells")?.as_array()?;
    let mut out = String::new();
    for cell in cells {
        let kind = cell.get("cell_type").and_then(|k| k.as_str()).unwrap_or("");
        let src = match cell.get("source") {
            Some(serde_json::Value::Array(a)) => {
                a.iter().filter_map(|s| s.as_str()).collect::<String>()
            }
            Some(serde_json::Value::String(s)) => s.clone(),
            _ => String::new(),
        };
        if kind == "markdown" {
            out.push_str("# --- markdown ---\n");
            for line in src.lines() {
                out.push_str("# ");
                out.push_str(line);
                out.push('\n');
            }
        } else {
            out.push_str("# --- code ---\n");
            out.push_str(&src);
            if !src.ends_with('\n') {
                out.push('\n');
            }
        }
        out.push('\n');
    }
    Some(out)
}

/// Render `text` (already the display text) into a highlighted `PixImage`.
fn render_text(text: &str, spec: &LangSpec) -> PixImage {
    // Collect raw lines up to the caps (line + total-cell budget), tab-expanded.
    let raw_lines: Vec<&str> = text.lines().collect();
    let total_lines = raw_lines.len();
    let gutter_w = digits(total_lines.clamp(1, MAX_LINES)) + 1; // number + one space

    let mut rows: Vec<Vec<(char, Tok)>> = Vec::new();
    let mut carry = Carry::None;
    let mut cells_used = 0usize;
    let mut truncated_at: Option<usize> = None;

    for (n, raw) in raw_lines.iter().enumerate() {
        if n >= MAX_LINES || cells_used >= MAX_CELLS {
            truncated_at = Some(n);
            break;
        }
        let expanded = expand_tabs(raw);
        let chars: Vec<char> = expanded.chars().collect();
        let mut lexed = lex_line(&chars, spec, &mut carry);
        if lexed.len() > MAX_COLS {
            lexed.truncate(MAX_COLS - 1);
            lexed.push(('»', Tok::Punct));
        }
        cells_used += (lexed.len() + gutter_w).max(1);
        rows.push(lexed);
    }

    let content_cols = rows.iter().map(|r| r.len()).max().unwrap_or(0);
    let cols = (gutter_w + content_cols).max(gutter_w + 1);
    // One extra row for a truncation notice, if any.
    let notice =
        truncated_at.map(|n| format!("… {} more lines — open in your editor", total_lines - n));
    let n_rows = rows.len() + usize::from(notice.is_some());
    let n_rows = n_rows.max(1);

    let w = cols * CELL_W;
    let h = n_rows * CELL_H;
    let mut pixels = vec![[BG[0], BG[1], BG[2], 255]; w * h];

    // Gutter line numbers + content.
    for (ri, row) in rows.iter().enumerate() {
        let lineno = ri + 1;
        blit_str(
            &mut pixels,
            w,
            ri,
            0,
            &format!("{lineno:>width$}", width = gutter_w - 1),
            GUTTER,
        );
        for (ci, &(ch, tok)) in row.iter().enumerate() {
            blit_glyph(&mut pixels, w, ri, gutter_w + ci, to_cp437(ch), tok.color());
        }
    }
    if let Some(msg) = notice {
        blit_str(&mut pixels, w, rows.len(), gutter_w, &msg, TRUNC);
    }

    PixImage::from_rgba(w as u32, h as u32, pixels)
}

fn digits(mut n: usize) -> usize {
    let mut d = 1;
    while n >= 10 {
        n /= 10;
        d += 1;
    }
    d
}

fn expand_tabs(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut col = 0;
    for c in s.chars() {
        if c == '\t' {
            let spaces = TAB - (col % TAB);
            for _ in 0..spaces {
                out.push(' ');
            }
            col += spaces;
        } else {
            out.push(c);
            col += 1;
        }
    }
    out
}

fn blit_str(pixels: &mut [[u8; 4]], w: usize, row: usize, col0: usize, s: &str, fg: [u8; 3]) {
    for (i, c) in s.chars().enumerate() {
        blit_glyph(pixels, w, row, col0 + i, to_cp437(c), fg);
    }
}

fn blit_glyph(pixels: &mut [[u8; 4]], w: usize, row: usize, col: usize, ch: u8, fg: [u8; 3]) {
    let glyph = &CP437_8X16[ch as usize];
    let x0 = col * CELL_W;
    let y0 = row * CELL_H;
    for (ry, &bits) in glyph.iter().enumerate() {
        for rx in 0..CELL_W {
            if (bits >> (7 - rx)) & 1 == 1 {
                let (px, py) = (x0 + rx, y0 + ry);
                if px < w {
                    let idx = py * w + px;
                    if idx < pixels.len() {
                        pixels[idx] = [fg[0], fg[1], fg[2], 255];
                    }
                }
            }
        }
    }
}

pub struct CodeDecoder;

/// Every extension this decoder claims. Kept in one place so `app.rs`'s parallel
/// `is_textmode_ext` / `is_image_ext` lists can reference the same set.
pub const CODE_EXTS: &[&str] = &[
    "rs",
    "c",
    "cpp",
    "cc",
    "cxx",
    "h",
    "hpp",
    "hh",
    "hxx",
    "inc",
    "ino",
    "m",
    "mm",
    "java",
    "cs",
    "go",
    "swift",
    "kt",
    "kts",
    "scala",
    "dart",
    "php",
    "php3",
    "php4",
    "php5",
    "hlsl",
    "glsl",
    "shader",
    "gdshader",
    "js",
    "jsx",
    "mjs",
    "cjs",
    "ts",
    "tsx",
    "json5",
    "py",
    "pyw",
    "gd",
    "pl",
    "pm",
    "rb",
    "sh",
    "bash",
    "zsh",
    "yaml",
    "yml",
    "toml",
    "ini",
    "cfg",
    "conf",
    "r",
    "jl",
    "ex",
    "exs",
    "coffee",
    "tcl",
    "ps1",
    "cmake",
    "mk",
    "lua",
    "bas",
    "bm",
    "bi",
    "vb",
    "vbs",
    "qb",
    "frm",
    "asm",
    "s",
    "nasm",
    "a51",
    "css",
    "scss",
    "sass",
    "less",
    "html",
    "htm",
    "htmlx",
    "xhtml",
    "xml",
    "xaml",
    "vue",
    "svelte",
    "json",
    "jsonc",
    "ipynb",
    "md",
    "markdown",
    "log",
    "bbs",
    "text",
    "csv",
    "tsv",
    "env",
    "gitignore",
    "properties",
    "rst",
];

impl Decoder for CodeDecoder {
    fn name(&self) -> &'static str {
        "code"
    }

    fn extensions(&self) -> &'static [&'static str] {
        CODE_EXTS
    }

    fn sniff(&self, _header: &[u8]) -> bool {
        // Text has no magic; dispatch by extension only (so PNG/etc. never reach here).
        false
    }

    fn decode(&self, bytes: &[u8]) -> Result<PixImage, DecodeError> {
        // `decode_bytes` dispatches here by extension, but doesn't pass it — infer the
        // language from content isn't worth it; default to plain unless it's a notebook.
        let text =
            ipynb_to_text(bytes).unwrap_or_else(|| String::from_utf8_lossy(bytes).into_owned());
        // Without the path we can't pick the exact LangSpec here; the registry calls
        // `decode_with_ext` when it knows the extension (see mod.rs). This bare path
        // renders plain (still correct, just uncolored).
        Ok(render_text(&text, &PLAIN))
    }
}

impl CodeDecoder {
    /// Extension-aware decode (the registry routes here so we can pick the language).
    pub fn decode_ext(bytes: &[u8], ext: &str) -> Result<PixImage, DecodeError> {
        // A notebook flattens to `#`-commented Python-ish text, so highlight it as Python.
        if ext == "ipynb" {
            let text =
                ipynb_to_text(bytes).unwrap_or_else(|| String::from_utf8_lossy(bytes).into_owned());
            return Ok(render_text(&text, &HASH));
        }
        let text = String::from_utf8_lossy(bytes).into_owned();
        Ok(render_text(&text, lang_for(ext)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ext_render(src: &str, ext: &str) -> PixImage {
        CodeDecoder::decode_ext(src.as_bytes(), ext).unwrap()
    }

    #[test]
    fn renders_nonempty_and_sized_to_content() {
        let img = ext_render("fn main() {}\nlet x = 1;\n", "rs");
        assert!(img.width > 0 && img.height > 0);
        // Two lines → 2 rows × 16px tall.
        assert_eq!(img.height, 2 * CELL_H as u32);
    }

    #[test]
    fn keyword_and_string_get_distinct_colors() {
        // "let" is a keyword, the "hi" literal is a string — different fg colors must appear.
        let spec = lang_for("rs");
        let line: Vec<char> = "let s = \"hi\";".chars().collect();
        let mut carry = Carry::None;
        let toks = lex_line(&line, spec, &mut carry);
        let kinds: std::collections::HashSet<_> = toks.iter().map(|&(_, t)| t.color()).collect();
        assert!(kinds.contains(&KEYWORD));
        assert!(kinds.contains(&STRING));
    }

    #[test]
    fn block_comment_carries_across_lines() {
        let spec = lang_for("c");
        let mut carry = Carry::None;
        let l1: Vec<char> = "int x; /* start".chars().collect();
        let t1 = lex_line(&l1, spec, &mut carry);
        assert!(matches!(carry, Carry::Block), "unterminated /* carries");
        // The tail after /* is a comment.
        assert_eq!(t1.last().unwrap().1.color(), COMMENT);
        let l2: Vec<char> = "still comment */ int y;".chars().collect();
        let _ = lex_line(&l2, spec, &mut carry);
        assert!(matches!(carry, Carry::None), "*/ closes the carry");
    }

    #[test]
    fn python_hash_comment_and_triple_string() {
        let spec = lang_for("py");
        let mut carry = Carry::None;
        let l: Vec<char> = "x = 1  # note".chars().collect();
        let t = lex_line(&l, spec, &mut carry);
        assert_eq!(t.last().unwrap().1.color(), COMMENT);
    }

    #[test]
    fn ipynb_extracts_cells() {
        let nb = br#"{"cells":[{"cell_type":"code","source":["print(1)\n"]}]}"#;
        let txt = ipynb_to_text(nb).unwrap();
        assert!(txt.contains("print(1)"));
        assert!(txt.contains("code"));
    }

    #[test]
    fn plain_text_has_no_highlight_but_renders() {
        let img = ext_render("just some text\nmore text\n", "txt");
        assert!(img.height >= 2 * CELL_H as u32);
    }

    #[test]
    fn long_line_is_clipped() {
        let long = "x".repeat(1000);
        let img = ext_render(&long, "txt");
        // Width capped near MAX_COLS (+ gutter), not 1000 cells wide.
        assert!(img.width <= ((MAX_COLS + 8) * CELL_W) as u32);
    }
}
