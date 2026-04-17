use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::Result;
use tokio::process::Command;
use tokio::time::timeout;

use crate::types::Language;

pub mod runner;
pub mod seccomp;

/// サンドボックス実行の設定
#[derive(Debug, Clone)]
pub struct SandboxConfig {
    pub time_limit: Duration,
    /// 標準出力の最大バイト数（超えたら切り捨て）
    pub max_output_bytes: usize,
    /// RLIMIT_AS の上限。None = 制限なし（インタプリタ言語向け）
    pub vm_limit_bytes: Option<u64>,
}

/// `compile()` の結果
pub struct CompileOutput {
    pub executable: PathBuf,
    pub run_args: Vec<String>,
    /// Some(msg) = CE、None = 成功
    pub error: Option<String>,
    /// 成功時のコンパイラ出力（警告など）
    pub warnings: String,
}

/// サンドボックス内実行の結果
#[derive(Debug)]
pub struct RunResult {
    pub stdout: Vec<u8>,
    pub stderr: Vec<u8>,
    pub exit_code: Option<i32>,
    pub time_used: Duration,
    /// getrusage(RUSAGE_CHILDREN).max_rss から取得（バイト単位）
    pub memory_used_bytes: u64,
    pub status: RunStatus,
}

#[derive(Debug)]
#[allow(dead_code)]
pub enum RunStatus {
    Ok,
    TimeLimitExceeded,
    MemoryLimitExceeded,
    RuntimeError,
    /// シグナルで強制終了（シグナル番号）
    Killed(i32),
}

/// ソースコードをコンパイル（またはシンタックスチェック）して実行に必要な情報を返す。
pub async fn compile(
    source_code: &str,
    language: &Language,
    work_dir: &Path,
) -> Result<CompileOutput> {
    let src_path = work_dir.join(format!("solution.{}", language.extension()));
    tokio::fs::write(&src_path, source_code).await?;

    if language.is_interpreted() {
        let interp = language.interpreter();

        // `which` で絶対パスを解決する。PATH の曖昧さを排除し、
        // macOS のスタブ (/usr/bin/python3) を誤って使わないようにする。
        let interp_path = resolve_interpreter(interp).await?;

        let result = timeout(
            Duration::from_secs(10),
            Command::new(&interp_path)
                .args(["-m", "py_compile", src_path.to_str().unwrap()])
                .output(),
        )
        .await;

        let (error, warnings) = match result {
            Err(_) => (Some("構文チェックがタイムアウトしました".to_string()), String::new()),
            Ok(Err(e)) => (Some(format!("インタプリタの起動に失敗しました: {e}")), String::new()),
            Ok(Ok(out)) => {
                let stderr = String::from_utf8_lossy(&out.stderr).into_owned();
                if out.status.success() {
                    (None, stderr)
                } else {
                    (Some(stderr), String::new())
                }
            }
        };

        Ok(CompileOutput {
            executable: interp_path,
            run_args: vec![src_path.to_str().unwrap().to_string()],
            error,
            warnings,
        })
    } else {
        let output_path = work_dir.join("solution");
        let args = language.compile_args(
            src_path.to_str().unwrap(),
            output_path.to_str().unwrap(),
        );

        let result = timeout(
            Duration::from_secs(30),
            Command::new(language.compiler()).args(&args).output(),
        )
        .await;

        match result {
            Err(_) => anyhow::bail!("コンパイルが30秒でタイムアウトしました"),
            Ok(Err(e)) => anyhow::bail!("コンパイラの起動に失敗しました: {e}"),
            Ok(Ok(out)) => {
                let stderr = String::from_utf8_lossy(&out.stderr).into_owned();
                if out.status.success() {
                    Ok(CompileOutput { executable: output_path, run_args: vec![], error: None, warnings: stderr })
                } else {
                    Ok(CompileOutput { executable: output_path, run_args: vec![], error: Some(stderr), warnings: String::new() })
                }
            }
        }
    }
}

/// `which <name>` で絶対パスを解決する。見つからなければエラー。
async fn resolve_interpreter(name: &str) -> Result<PathBuf> {
    let out = Command::new("which").arg(name).output().await?;
    anyhow::ensure!(out.status.success(), "interpreter '{name}' not found in PATH");
    let path = String::from_utf8(out.stdout)?.trim().to_string();
    Ok(PathBuf::from(path))
}

/// 実行ファイルをサンドボックス内で実行して結果を返す。
///
/// `run_args`: execvp の argv[1..] に渡す引数（インタプリタ言語でソースパスを渡すのに使う）
pub async fn run_in_sandbox(
    executable: &Path,
    run_args: Vec<String>,
    stdin: &[u8],
    config: SandboxConfig,
) -> Result<RunResult> {
    let executable = executable.to_owned();
    let stdin = stdin.to_owned();

    tokio::task::spawn_blocking(move || {
        runner::run_sandboxed_blocking(&executable, &run_args, &stdin, &config)
    })
    .await?
}
