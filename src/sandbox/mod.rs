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
    /// `RLIMIT_NPROC` の上限。None = 制限しない
    pub nproc_limit: Option<u64>,
    /// Java / Go など seccomp ホワイトリストとの差分が大きいランタイムでは無効化する
    pub enable_seccomp: bool,
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
    /// 表示用の実行時間。TLE 判定と揃えるため wall time を使う。
    pub wall_time_used: Duration,
    /// getrusage(RUSAGE_CHILDREN).max_rss から取得（バイト単位）
    pub memory_used_bytes: u64,
    pub status: RunStatus,
}

#[derive(Debug)]
pub enum RunStatus {
    Ok,
    TimeLimitExceeded,
    RuntimeError,
    /// シグナルで強制終了
    Killed,
}

/// ソースコードをコンパイル（またはシンタックスチェック）して実行に必要な情報を返す。
pub async fn compile(
    source_code: &str,
    language: &Language,
    work_dir: &Path,
) -> Result<CompileOutput> {
    let src_path = work_dir.join(language.source_file_name());
    tokio::fs::write(&src_path, source_code).await?;

    if matches!(language, Language::Text) {
        return Ok(CompileOutput {
            executable: resolve_command("cat").await?,
            run_args: vec![],
            error: None,
            warnings: String::new(),
        });
    }

    if language.is_interpreted() {
        let interp = language.interpreter();

        // `which` で絶対パスを解決する。PATH の曖昧さを排除し、
        // macOS のスタブ (/usr/bin/python3) を誤って使わないようにする。
        let interp_path = resolve_command(interp).await?;

        let result = timeout(
            Duration::from_secs(10),
            Command::new(&interp_path)
                .args(["-m", "py_compile", src_path.to_str().unwrap()])
                .output(),
        )
        .await;

        let (error, warnings) = match result {
            Err(_) => (
                Some("構文チェックがタイムアウトしました".to_string()),
                String::new(),
            ),
            Ok(Err(e)) => (
                Some(format!("インタプリタの起動に失敗しました: {e}")),
                String::new(),
            ),
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
    } else if matches!(language, Language::Java) {
        let args = language.compile_args(src_path.to_str().unwrap(), "");
        let result = timeout(
            Duration::from_secs(30),
            Command::new(language.compiler())
                .current_dir(work_dir)
                .args(&args)
                .output(),
        )
        .await;

        match result {
            Err(_) => anyhow::bail!("コンパイルが30秒でタイムアウトしました"),
            Ok(Err(e)) => anyhow::bail!("コンパイラの起動に失敗しました: {e}"),
            Ok(Ok(out)) => {
                let stderr = String::from_utf8_lossy(&out.stderr).into_owned();
                if out.status.success() {
                    Ok(CompileOutput {
                        executable: resolve_command("java").await?,
                        run_args: vec![
                            "-cp".to_string(),
                            work_dir.to_str().unwrap().to_string(),
                            "Main".to_string(),
                        ],
                        error: None,
                        warnings: stderr,
                    })
                } else {
                    Ok(CompileOutput {
                        executable: resolve_command("java").await?,
                        run_args: vec![],
                        error: Some(stderr),
                        warnings: String::new(),
                    })
                }
            }
        }
    } else {
        let output_path = work_dir.join("solution");
        let args = language.compile_args(src_path.to_str().unwrap(), output_path.to_str().unwrap());

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
                    Ok(CompileOutput {
                        executable: output_path,
                        run_args: vec![],
                        error: None,
                        warnings: stderr,
                    })
                } else {
                    Ok(CompileOutput {
                        executable: output_path,
                        run_args: vec![],
                        error: Some(stderr),
                        warnings: String::new(),
                    })
                }
            }
        }
    }
}

async fn resolve_command(name: &str) -> Result<PathBuf> {
    let path = std::env::var_os("PATH").ok_or_else(|| anyhow::anyhow!("PATH is not set"))?;
    let candidates: Vec<PathBuf> = std::env::split_paths(&path)
        .map(|dir| dir.join(name))
        .filter(|candidate| candidate.is_file())
        .collect();

    #[cfg(target_os = "macos")]
    if let Some(found) = candidates
        .iter()
        .find(|candidate| !candidate.starts_with("/usr/bin"))
        .cloned()
    {
        return Ok(found);
    }

    if let Some(found) = candidates.into_iter().next() {
        return Ok(found);
    }

    let out = Command::new("which").arg(name).output().await?;
    anyhow::ensure!(out.status.success(), "command '{name}' not found in PATH");
    Ok(PathBuf::from(String::from_utf8(out.stdout)?.trim()))
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
