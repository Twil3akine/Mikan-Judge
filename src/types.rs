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
pub struct Submission {
    pub id: Uuid,
    pub source_code: String,
    pub language: Language,
    pub problem_id: String,
    pub status: JudgeStatus,
    pub time_used_ms: Option<u64>,
    pub memory_used_kb: Option<u64>,
    pub stdout: Option<String>,
    pub stderr: Option<String>,
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
