use std::path::{Path, PathBuf};

use anyhow::{bail, Result};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ProblemMeta {
    pub title: String,
    #[serde(default = "default_time_limit")]
    pub time_limit_ms: u64,
    #[serde(default = "default_memory_limit")]
    pub memory_limit_kb: u64,
}

fn default_time_limit() -> u64 { 2000 }
fn default_memory_limit() -> u64 { 262144 }

#[derive(Debug, Clone, Serialize)]
pub struct Problem {
    pub id: String,
    pub title: String,
    pub time_limit_ms: u64,
    pub memory_limit_kb: u64,
    pub html_content: String,
    #[serde(skip)]
    pub testcases: Vec<Testcase>,
}

#[derive(Debug, Clone)]
pub struct Testcase {
    pub input: String,
    pub expected: String,
}

pub fn load_all(problems_dir: &Path) -> Vec<Problem> {
    let Ok(entries) = std::fs::read_dir(problems_dir) else {
        return Vec::new();
    };

    let mut dirs: Vec<_> = entries
        .filter_map(|e| e.ok())
        .filter(|e| e.path().is_dir())
        .collect();
    dirs.sort_by_key(|e| e.file_name());

    dirs.iter()
        .filter_map(|e| {
            let id = e.file_name().to_string_lossy().to_string();
            load_one(problems_dir, &id).ok()
        })
        .collect()
}

pub fn load_one(problems_dir: &Path, id: &str) -> Result<Problem> {
    let dir = problems_dir.join(id);

    let meta: ProblemMeta =
        toml::from_str(&std::fs::read_to_string(dir.join("meta.toml"))?)?;

    let md = std::fs::read_to_string(dir.join("statement.md"))?;
    let html_content = markdown_to_html(&md);

    let testcases = load_testcases(&dir.join("testcases"))?;
    if testcases.is_empty() {
        bail!("problem '{id}' has no test cases");
    }

    Ok(Problem {
        id: id.to_string(),
        title: meta.title,
        time_limit_ms: meta.time_limit_ms,
        memory_limit_kb: meta.memory_limit_kb,
        html_content,
        testcases,
    })
}

fn markdown_to_html(md: &str) -> String {
    use pulldown_cmark::{html, Options, Parser};
    let mut opts = Options::empty();
    opts.insert(Options::ENABLE_TABLES);
    opts.insert(Options::ENABLE_STRIKETHROUGH);
    let mut out = String::new();
    html::push_html(&mut out, Parser::new_ext(md, opts));
    out
}

fn load_testcases(dir: &Path) -> Result<Vec<Testcase>> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Ok(Vec::new());
    };

    let mut in_files: Vec<PathBuf> = entries
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| p.extension().map_or(false, |ext| ext == "in"))
        .collect();
    in_files.sort();

    in_files
        .iter()
        .filter_map(|in_path| {
            let out_path = in_path.with_extension("out");
            if out_path.exists() {
                let input = std::fs::read_to_string(in_path).ok()?;
                let expected = std::fs::read_to_string(&out_path).ok()?;
                Some(Ok(Testcase { input, expected }))
            } else {
                None
            }
        })
        .collect()
}
