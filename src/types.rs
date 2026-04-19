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
}

impl Language {
    pub fn to_db(&self) -> &'static str {
        match self {
            Language::Cpp => "cpp",
            Language::Rust => "rust",
            Language::Python => "python",
            Language::PyPy => "pypy",
        }
    }

    pub fn from_db(s: &str) -> Self {
        match s {
            "rust" => Language::Rust,
            "python" => Language::Python,
            "pypy" => Language::PyPy,
            _ => Language::Cpp,
        }
    }

    pub fn extension(&self) -> &'static str {
        match self {
            Language::Cpp => "cpp",
            Language::Rust => "rs",
            Language::Python | Language::PyPy => "py",
        }
    }

    pub fn display_name_versioned(&self, versions: &LanguageVersions) -> String {
        match self {
            Language::Cpp => format!("C++17 (GCC {})", versions.cpp),
            Language::Rust => format!("Rust ({})", versions.rust),
            Language::Python => format!("Python (CPython {})", versions.python),
            Language::PyPy => format!("Python (PyPy {})", versions.pypy),
        }
    }

    pub fn is_interpreted(&self) -> bool {
        matches!(self, Language::Python | Language::PyPy)
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
}

impl LanguageVersions {
    pub async fn detect() -> Self {
        Self {
            cpp:    detect_version("g++",    &["--version"]).await.unwrap_or_else(|| "?".into()),
            rust:   detect_version("rustc",  &["--version"]).await.unwrap_or_else(|| "?".into()),
            python: detect_version("python3", &["--version"]).await.unwrap_or_else(|| "?".into()),
            pypy:   detect_version("pypy3",  &["--version"]).await.unwrap_or_else(|| "?".into()),
        }
    }
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
    let first_line = raw.lines().next()?.trim().to_string();
    Some(parse_version(cmd, &first_line))
}

fn parse_version(cmd: &str, line: &str) -> String {
    match cmd {
        // "Python 3.13.1" → "3.13.1"
        "python3" | "pypy3" => line
            .split_whitespace()
            .nth(1)
            .unwrap_or(line)
            .to_string(),
        // "rustc 1.82.0 (f6e511eec 2024-10-15)" → "1.82.0"
        "rustc" => line
            .split_whitespace()
            .nth(1)
            .unwrap_or(line)
            .to_string(),
        // "g++ (Homebrew GCC 14.2.0...) 14.2.0" or "g++ (GCC) 14.2.0" → last word
        "g++" => line
            .split_whitespace()
            .last()
            .unwrap_or(line)
            .to_string(),
        _ => line.to_string(),
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
            "compile_error" => JudgeStatus::CompileError { message: String::new() },
            "internal_error" => JudgeStatus::InternalError { message: String::new() },
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
    RuntimeError { exit_code: i32 },
    CompileError { message: String },
    InternalError { message: String },
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
}

/// テストケース1件の実行結果
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TestcaseVerdict {
    pub verdict: String,
    pub time_ms: Option<u64>,
    pub memory_kb: Option<u64>,
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
            ContestStatus::Ongoing  => "開催中",
            ContestStatus::Past     => "終了",
        }
    }
    pub fn badge_class(&self) -> &'static str {
        match self {
            ContestStatus::Upcoming => "pending",
            ContestStatus::Ongoing  => "ac",
            ContestStatus::Past     => "ce",
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
