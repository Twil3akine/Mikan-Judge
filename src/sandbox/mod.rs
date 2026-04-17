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
    pub memory_limit_bytes: u64,
    /// 標準出力の最大バイト数（超えたら切り捨て）
    pub max_output_bytes: usize,
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
///
/// 戻り値: `(実行コマンド, 追加引数, コンパイルエラー)`
/// - コンパイル言語: `("work_dir/solution", [], None/Some(err))`
/// - インタプリタ言語: `("python3", ["work_dir/solution.py"], None/Some(err))`
pub async fn compile(
    source_code: &str,
    language: &Language,
    work_dir: &Path,
) -> Result<(PathBuf, Vec<String>, Option<String>)> {
    let src_path = work_dir.join(format!("solution.{}", language.extension()));
    tokio::fs::write(&src_path, source_code).await?;

    if language.is_interpreted() {
        let interp = language.interpreter();
        // 構文チェック（py_compile）
        let result = timeout(
            Duration::from_secs(10),
            Command::new(interp)
                .args(["-m", "py_compile", src_path.to_str().unwrap()])
                .output(),
        )
        .await;

        let compile_err = match result {
            Err(_) => Some("構文チェックがタイムアウトしました".to_string()),
            Ok(Err(e)) => Some(format!("インタプリタの起動に失敗しました: {e}")),
            Ok(Ok(out)) => {
                if out.status.success() {
                    None
                } else {
                    Some(String::from_utf8_lossy(&out.stderr).into_owned())
                }
            }
        };

        Ok((
            PathBuf::from(interp),
            vec![src_path.to_str().unwrap().to_string()],
            compile_err,
        ))
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
                if out.status.success() {
                    Ok((output_path, vec![], None))
                } else {
                    let stderr = String::from_utf8_lossy(&out.stderr).into_owned();
                    Ok((output_path, vec![], Some(stderr)))
                }
            }
        }
    }
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
