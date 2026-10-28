use std::fs;
use std::path::{Path, PathBuf};

// (file suffix, class, reason): literals that are intentionally theme-independent.
const ALLOWLIST: &[(&str, &str, &str)] = &[];

const LIGHT_BG: &[&str] = &[
    "white",
    "slate-50",
    "slate-100",
    "slate-200",
    "indigo-50",
    "indigo-100",
];
const LIGHT_TEXT: &[&str] = &["slate-600", "slate-700", "slate-800", "slate-900"];
const LIGHT_BORDER: &[&str] = &["slate-100", "slate-200", "slate-300"];

fn rust_files(dir: &Path, out: &mut Vec<PathBuf>) {
    for entry in fs::read_dir(dir).expect("source directory is readable") {
        let path = entry.expect("directory entry").path();
        if path.is_dir() {
            rust_files(&path, out);
        } else if path.extension().is_some_and(|ext| ext == "rs") {
            out.push(path);
        }
    }
}

/// Returns (1-based line, content) for every string literal in `src`, skipping comments and char literals.
fn string_literals(src: &str) -> Vec<(usize, String)> {
    let chars: Vec<char> = src.chars().collect();
    let mut out = Vec::new();
    let (mut i, mut line) = (0, 1);
    while i < chars.len() {
        let c = chars[i];
        match c {
            '\n' => {
                line += 1;
                i += 1;
            }
            '/' if chars.get(i + 1) == Some(&'/') => {
                while i < chars.len() && chars[i] != '\n' {
                    i += 1;
                }
            }
            '/' if chars.get(i + 1) == Some(&'*') => {
                let mut depth = 0;
                while i < chars.len() {
                    if chars[i] == '/' && chars.get(i + 1) == Some(&'*') {
                        depth += 1;
                        i += 2;
                    } else if chars[i] == '*' && chars.get(i + 1) == Some(&'/') {
                        depth -= 1;
                        i += 2;
                        if depth == 0 {
                            break;
                        }
                    } else {
                        if chars[i] == '\n' {
                            line += 1;
                        }
                        i += 1;
                    }
                }
            }
            'r' if matches!(chars.get(i + 1), Some('"' | '#'))
                && (i == 0 || !(chars[i - 1].is_alphanumeric() || chars[i - 1] == '_')) =>
            {
                let mut j = i + 1;
                let mut hashes = 0;
                while chars.get(j) == Some(&'#') {
                    hashes += 1;
                    j += 1;
                }
                if chars.get(j) != Some(&'"') {
                    i += 1;
                    continue;
                }
                j += 1;
                let start_line = line;
                let mut text = String::new();
                while j < chars.len() {
                    if chars[j] == '"' && (0..hashes).all(|k| chars.get(j + 1 + k) == Some(&'#')) {
                        break;
                    }
                    if chars[j] == '\n' {
                        line += 1;
                    }
                    text.push(chars[j]);
                    j += 1;
                }
                out.push((start_line, text));
                i = j + 1 + hashes;
            }
            '"' => {
                let start_line = line;
                let mut text = String::new();
                i += 1;
                while i < chars.len() && chars[i] != '"' {
                    if chars[i] == '\\' {
                        i += 1;
                    }
                    if chars.get(i) == Some(&'\n') {
                        line += 1;
                    }
                    if let Some(ch) = chars.get(i) {
                        text.push(*ch);
                    }
                    i += 1;
                }
                out.push((start_line, text));
                i += 1;
            }
            '\'' => {
                // Char literal ('"', '\'') or lifetime: skip a literal so its quote is not read as a string.
                if chars.get(i + 1) == Some(&'\\') {
                    i += 2;
                    while i < chars.len() && chars[i] != '\'' {
                        i += 1;
                    }
                    i += 1;
                } else if chars.get(i + 2) == Some(&'\'') {
                    i += 3;
                } else {
                    i += 1;
                }
            }
            _ => i += 1,
        }
    }
    out
}

fn family(class: &str) -> Option<&'static str> {
    let (family, colours) = if let Some(rest) = class.strip_prefix("bg-") {
        ("bg-", (rest, LIGHT_BG))
    } else if let Some(rest) = class.strip_prefix("text-") {
        ("text-", (rest, LIGHT_TEXT))
    } else if let Some(rest) = class.strip_prefix("border-") {
        ("border-", (rest, LIGHT_BORDER))
    } else {
        return None;
    };
    let (colour, light) = colours;
    let colour = colour.split('/').next().unwrap_or(colour);
    light.contains(&colour).then_some(family)
}

fn utility(token: &str) -> &str {
    token.rsplit(':').next().unwrap_or(token)
}

/// The variants of `token` other than `dark`, in a stable order: `hover:md:` and `md:hover:`
/// are the same state.
fn states(token: &str) -> Vec<&str> {
    let mut parts: Vec<&str> = token.split(':').collect();
    parts.pop();
    parts.retain(|part| *part != "dark");
    parts.sort_unstable();
    parts
}

/// Light colour classes without a `dark:` class of the same family for the same state.
fn violations(literal: &str) -> Vec<String> {
    let tokens: Vec<&str> = literal.split_whitespace().collect();
    tokens
        .iter()
        .filter(|token| !token.split(':').any(|part| part == "dark"))
        .filter_map(|token| family(utility(token)).map(|family| (*token, family)))
        .filter(|(token, family)| {
            !tokens.iter().any(|other| {
                other.split(':').any(|part| part == "dark")
                    && utility(other).starts_with(family)
                    && states(other) == states(token)
            })
        })
        .map(|(token, _)| token.to_owned())
        .collect()
}

#[test]
fn light_colour_classes_have_a_dark_counterpart() {
    let src = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut files = Vec::new();
    rust_files(&src, &mut files);
    files.sort();
    let mut failures = Vec::new();
    for file in files {
        let relative = file
            .strip_prefix(&src)
            .unwrap()
            .to_string_lossy()
            .replace('\\', "/");
        let source = fs::read_to_string(&file).expect("source file is readable");
        for (line, literal) in string_literals(&source) {
            for class in violations(&literal) {
                let allowed = ALLOWLIST
                    .iter()
                    .any(|(suffix, allowed, _)| relative.ends_with(suffix) && *allowed == class);
                if !allowed {
                    failures.push(format!(
                        "src/{relative}:{line}: `{class}` has no dark: counterpart"
                    ));
                }
            }
        }
    }
    assert!(
        failures.is_empty(),
        "missing dark variants:\n{}",
        failures.join("\n")
    );
}

#[test]
fn scanner_understands_rust_literals() {
    let source = "// \"bg-white\" in a comment\nconst A: &str = \"bg-white dark:bg-slate-900\";\nlet q = '\"';\nlet r = r#\"text-slate-700 \"x\"\"#;\n";
    let literals = string_literals(source);
    assert_eq!(literals.len(), 2, "{literals:?}");
    assert_eq!(literals[0].0, 2);
    assert!(violations(&literals[0].1).is_empty());
    assert_eq!(literals[1].0, 4);
    assert_eq!(violations(&literals[1].1), ["text-slate-700"]);
    assert_eq!(
        violations("hover:bg-slate-100 dark:hover:bg-slate-800"),
        Vec::<String>::new()
    );
    assert_eq!(
        violations("hover:bg-slate-100 dark:bg-slate-800"),
        ["hover:bg-slate-100"]
    );
    assert_eq!(
        violations("md:hover:bg-white dark:hover:md:bg-slate-900"),
        Vec::<String>::new()
    );
    assert_eq!(
        violations("border-slate-200 dark:bg-slate-800"),
        ["border-slate-200"]
    );
}
