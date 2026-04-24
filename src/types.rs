use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Language {
    Cpp,
    Rust,
    Python,
    PyPy,
    Java,
    Go,
    Text,
}

impl Language {
    pub fn to_db(&self) -> &'static str {
        match self {
            Language::Cpp => "cpp",
            Language::Rust => "rust",
            Language::Python => "python",
            Language::PyPy => "pypy",
            Language::Java => "java",
            Language::Go => "go",
            Language::Text => "text",
        }
    }

    pub fn from_db(s: &str) -> Self {
        match s {
            "rust" => Language::Rust,
            "python" => Language::Python,
            "pypy" => Language::PyPy,
            "java" => Language::Java,
            "go" => Language::Go,
            "text" => Language::Text,
            _ => Language::Cpp,
        }
    }

    pub fn display_name_versioned(&self, versions: &LanguageVersions) -> String {
        match self {
            Language::Cpp => format!("C++17 (GCC {})", versions.cpp),
            Language::Rust => format!("Rust (rustc {})", versions.rust),
            Language::Python => format!("Python (CPython {})", versions.python),
            Language::PyPy => format!("Python (PyPy {})", versions.pypy),
            Language::Java => format!("Java (OpenJDK {})", versions.java),
            Language::Go => format!("Go ({})", versions.go),
            Language::Text => format!("Text (cat {})", versions.text),
        }
    }

    pub fn is_interpreted(&self) -> bool {
        matches!(self, Language::Python | Language::PyPy)
    }

    pub fn needs_unlimited_vm(&self) -> bool {
        matches!(self, Language::Python | Language::PyPy | Language::Java)
    }

    pub fn needs_relaxed_seccomp(&self) -> bool {
        matches!(self, Language::Java | Language::Go | Language::Text)
    }

    pub fn needs_relaxed_nproc(&self) -> bool {
        matches!(self, Language::Java | Language::Go)
    }

    pub fn source_file_name(&self) -> &'static str {
        match self {
            Language::Java => "Main.java",
            Language::Text => "solution.txt",
            Language::Cpp => "solution.cpp",
            Language::Rust => "solution.rs",
            Language::Python | Language::PyPy => "solution.py",
            Language::Go => "solution.go",
        }
    }

    pub fn interpreter(&self) -> &'static str {
        match self {
            Language::Python => "python3",
            Language::PyPy => "pypy3",
            _ => panic!("not an interpreted language"),
        }
    }

    pub fn compiler(&self) -> &'static str {
        match self {
            Language::Cpp => "g++",
            Language::Rust => "rustc",
            Language::Java => "javac",
            Language::Go => "go",
            _ => panic!("not a compiled language"),
        }
    }

    pub fn compile_args(&self, source: &str, output: &str) -> Vec<String> {
        match self {
            Language::Cpp => vec![
                source.to_string(),
                "-o".to_string(),
                output.to_string(),
                "-O2".to_string(),
                "-std=c++17".to_string(),
            ],
            Language::Rust => vec![
                source.to_string(),
                "-o".to_string(),
                output.to_string(),
                "-C".to_string(),
                "opt-level=2".to_string(),
            ],
            Language::Java => vec![
                "-encoding".to_string(),
                "UTF-8".to_string(),
                source.to_string(),
            ],
            Language::Go => vec![
                "build".to_string(),
                "-o".to_string(),
                output.to_string(),
                source.to_string(),
            ],
            _ => panic!("not a compiled language"),
        }
    }
}

/// 各言語の実行環境バージョン（起動時に一度だけ検出してキャッシュする）
#[derive(Debug, Clone)]
pub struct LanguageVersions {
    pub cpp: String,
    pub rust: String,
    pub python: String,
    pub pypy: String,
    pub java: String,
    pub go: String,
    pub text: String,
}

impl LanguageVersions {
    pub async fn detect() -> Self {
        Self {
            cpp: detect_version("g++", &["--version"])
                .await
                .unwrap_or_else(|| "?".into()),
            rust: detect_version("rustc", &["--version"])
                .await
                .unwrap_or_else(|| "?".into()),
            python: detect_version("python3", &["--version"])
                .await
                .unwrap_or_else(|| "?".into()),
            pypy: detect_version("pypy3", &["--version"])
                .await
                .unwrap_or_else(|| "?".into()),
            java: detect_version("javac", &["--version"])
                .await
                .unwrap_or_else(|| "?".into()),
            go: detect_version("go", &["version"])
                .await
                .unwrap_or_else(|| "?".into()),
            text: detect_text_version().await.unwrap_or_else(|| "unknown".into()),
        }
    }
}

async fn detect_text_version() -> Option<String> {
    if let Some(version) = detect_version("cat", &["--version"]).await {
        return Some(version);
    }
    detect_version("gcat", &["--version"]).await
}

async fn detect_version(cmd: &str, args: &[&str]) -> Option<String> {
    let out = tokio::process::Command::new(cmd)
        .args(args)
        .output()
        .await
        .ok()?;
    // python3/pypy3 は --version を stderr に出す場合がある
    let raw = if out.stdout.is_empty() {
        String::from_utf8_lossy(&out.stderr).into_owned()
    } else {
        String::from_utf8_lossy(&out.stdout).into_owned()
    };
    Some(parse_version(cmd, &raw))
}

fn parse_version(cmd: &str, raw: &str) -> String {
    match cmd {
        // "Python 3.13.1" → "3.13.1"
        "python3" => raw
            .lines()
            .next()
            .unwrap_or(raw)
            .split_whitespace()
            .nth(1)
            .unwrap_or(raw.trim())
            .to_string(),
        // "Python 3.11.15 ...\n[PyPy 7.3.21 with ...]" → "7.3.21"
        "pypy3" => {
            let pypy = raw
                .lines()
                .find(|line| line.contains("[PyPy "))
                .and_then(|line| line.split_whitespace().nth(1));

            pypy.unwrap_or_else(|| raw.lines().next().unwrap_or(raw).trim()).to_string()
        }
        // "rustc 1.82.0 (f6e511eec 2024-10-15)" → "1.82.0"
        "rustc" => raw
            .lines()
            .next()
            .unwrap_or(raw)
            .split_whitespace()
            .nth(1)
            .unwrap_or(raw.trim())
            .to_string(),
        // "javac 21.0.8" → "21.0.8"
        "javac" => raw
            .lines()
            .next()
            .unwrap_or(raw)
            .split_whitespace()
            .nth(1)
            .unwrap_or(raw.trim())
            .to_string(),
        // "go version go1.24.2 darwin/arm64" → "1.24.2"
        "go" => raw
            .lines()
            .next()
            .unwrap_or(raw)
            .split_whitespace()
            .nth(2)
            .unwrap_or(raw.trim())
            .trim_start_matches("go")
            .to_string(),
        // "cat (GNU coreutils) 9.7" → "9.7"
        "cat" => raw
            .lines()
            .next()
            .unwrap_or(raw)
            .split_whitespace()
            .last()
            .unwrap_or(raw.trim())
            .to_string(),
        // "g++ (Homebrew GCC 14.2.0...) 14.2.0" or "g++ (GCC) 14.2.0" → last word
        "g++" => raw
            .lines()
            .next()
            .unwrap_or(raw)
            .split_whitespace()
            .last()
            .unwrap_or(raw.trim())
            .to_string(),
        _ => raw.lines().next().unwrap_or(raw).trim().to_string(),
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum JudgeType {
    Exact,
    Heuristic,
}

impl JudgeType {
    pub fn from_db(s: &str) -> Self {
        match s {
            "heuristic" => JudgeType::Heuristic,
            _ => JudgeType::Exact,
        }
    }
}

impl JudgeStatus {
    pub fn to_db(&self) -> &'static str {
        match self {
            JudgeStatus::Pending => "pending",
            JudgeStatus::Running => "running",
            JudgeStatus::Accepted => "accepted",
            JudgeStatus::WrongAnswer => "wrong_answer",
            JudgeStatus::TimeLimitExceeded => "time_limit_exceeded",
            JudgeStatus::MemoryLimitExceeded => "memory_limit_exceeded",
            JudgeStatus::RuntimeError { .. } => "runtime_error",
            JudgeStatus::CompileError { .. } => "compile_error",
            JudgeStatus::InternalError { .. } => "internal_error",
            JudgeStatus::Scored => "scored",
        }
    }

    pub fn from_db(s: &str) -> Self {
        match s {
            "running" => JudgeStatus::Running,
            "accepted" => JudgeStatus::Accepted,
            "wrong_answer" => JudgeStatus::WrongAnswer,
            "time_limit_exceeded" => JudgeStatus::TimeLimitExceeded,
            "memory_limit_exceeded" => JudgeStatus::MemoryLimitExceeded,
            "runtime_error" => JudgeStatus::RuntimeError { exit_code: -1 },
            "compile_error" => JudgeStatus::CompileError {
                message: String::new(),
            },
            "internal_error" => JudgeStatus::InternalError {
                message: String::new(),
            },
            "scored" => JudgeStatus::Scored,
            _ => JudgeStatus::Pending,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum JudgeStatus {
    Pending,
    Running,
    Accepted,
    WrongAnswer,
    TimeLimitExceeded,
    MemoryLimitExceeded,
    RuntimeError {
        exit_code: i32,
    },
    CompileError {
        message: String,
    },
    InternalError {
        message: String,
    },
    /// ヒューリスティック: エラーなく実行完了（スコアは submissions.score に保存）
    Scored,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct User {
    pub id: Uuid,
    pub username: String,
    #[serde(skip_serializing)]
    pub password_hash: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Submission {
    pub id: Uuid,
    pub user_id: Option<Uuid>,
    pub contest_id: Option<String>,
    pub source_code: String,
    pub language: Language,
    pub problem_id: String,
    pub status: JudgeStatus,
    pub time_used_ms: Option<u64>,
    pub memory_used_kb: Option<u64>,
    pub stdout: Option<String>,
    pub stderr: Option<String>,
    /// 各テストケースの判定結果（verdict・実行時間・メモリ）
    pub testcase_results: Option<Vec<TestcaseVerdict>>,
    /// ヒューリスティック: テストケーススコアの合計
    pub score: Option<f64>,
}

/// テストケース1件の実行結果
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TestcaseVerdict {
    pub verdict: String,
    pub time_ms: Option<u64>,
    pub memory_kb: Option<u64>,
    /// ヒューリスティック: このケースのスコア
    #[serde(skip_serializing_if = "Option::is_none")]
    pub score: Option<f64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContestProblem {
    pub label: String,
    pub problem_id: String,
    pub display_order: i32,
}

/// コンテストのステータス（テンプレート表示用）
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub enum ContestStatus {
    Upcoming,
    Ongoing,
    Past,
}

impl ContestStatus {
    pub fn label(&self) -> &'static str {
        match self {
            ContestStatus::Upcoming => "予定",
            ContestStatus::Ongoing => "開催中",
            ContestStatus::Past => "終了",
        }
    }
    pub fn badge_class(&self) -> &'static str {
        match self {
            ContestStatus::Upcoming => "pending",
            ContestStatus::Ongoing => "ac",
            ContestStatus::Past => "ce",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Contest {
    pub id: String,
    pub title: String,
    pub description: String,
    pub start_time: DateTime<Utc>,
    pub end_time: DateTime<Utc>,
    pub judge_type: JudgeType,
}

impl Contest {
    pub fn status(&self) -> ContestStatus {
        let now = Utc::now();
        if now < self.start_time {
            ContestStatus::Upcoming
        } else if now <= self.end_time {
            ContestStatus::Ongoing
        } else {
            ContestStatus::Past
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SubmitRequest {
    pub source_code: String,
    pub language: Language,
    pub problem_id: String,
    /// テストケースの標準入力
    pub stdin: String,
    /// 期待される標準出力（ジャッジが比較する）
    pub expected_output: String,
    pub time_limit_ms: u64,
    pub memory_limit_kb: u64,
}
