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
        let compiler = resolve_command(language.compiler()).await?;
        let args = language.compile_args(src_path.to_str().unwrap(), "");
        let result = timeout(
            Duration::from_secs(30),
            Command::new(&compiler)
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
        let compiler = resolve_command(language.compiler()).await?;
        let output_path = work_dir.join("solution");
        let args = language.compile_args(src_path.to_str().unwrap(), output_path.to_str().unwrap());

        let result = timeout(
            Duration::from_secs(30),
            Command::new(&compiler).args(&args).output(),
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

#[cfg(test)]
mod tests {
    use super::{RunStatus, SandboxConfig, compile, run_in_sandbox};
    use crate::problem;
    use crate::sandbox::resolve_command;
    use crate::types::Language;
    use std::time::Duration;

    fn source_for(language: &Language) -> &'static str {
        match language {
            Language::Cpp => {
                "#include <iostream>\nint main(){long long a,b;std::cin>>a>>b;std::cout<<a+b<<\"\\n\";}\n"
            }
            Language::Rust => {
                "use std::io::{self, Read};\nfn main(){let mut s=String::new();io::stdin().read_to_string(&mut s).unwrap();let mut it=s.split_whitespace();let a:i64=it.next().unwrap().parse().unwrap();let b:i64=it.next().unwrap().parse().unwrap();println!(\"{}\",a+b);}\n"
            }
            Language::Python => {
                "a,b=map(int,input().split())\nprint(a+b)\n"
            }
            Language::PyPy => {
                "a,b=map(int,input().split())\nprint(a+b)\n"
            }
            Language::Java => {
                "import java.util.Scanner;\npublic class Main {\n    public static void main(String[] args) {\n        Scanner sc = new Scanner(System.in);\n        long a = sc.nextLong();\n        long b = sc.nextLong();\n        System.out.println(a + b);\n        sc.close();\n    }\n}\n"
            }
            Language::Go => {
                "package main\n\nimport \"fmt\"\n\nfunc main() {\n    var a, b int\n    fmt.Scan(&a, &b)\n    fmt.Println(a + b)\n}\n"
            }
            Language::Text => "5\n",
        }
    }

    fn sandbox_config(language: &Language, memory_limit_kb: u64, time_limit_ms: u64) -> SandboxConfig {
        let mem = memory_limit_kb * 1024;
        SandboxConfig {
            time_limit: Duration::from_millis(time_limit_ms),
            max_output_bytes: 16 * 1024 * 1024,
            vm_limit_bytes: if language.needs_unlimited_vm() {
                None
            } else {
                Some(mem * 2)
            },
            nproc_limit: if language.needs_relaxed_nproc() {
                None
            } else {
                Some(1)
            },
            enable_seccomp: !language.needs_relaxed_seccomp(),
        }
    }

    #[tokio::test]
    async fn smoke_aplusb_across_languages() {
        let required = ["g++", "rustc", "python3", "pypy3", "javac", "java", "go", "cat"];
        for command in required {
            if resolve_command(command).await.is_err() {
                eprintln!("skip smoke_aplusb_across_languages: command '{command}' not found in PATH");
                return;
            }
        }

        let problem = problem::load_one(std::path::Path::new("problems"), "aplusb")
            .expect("failed to load aplusb");
        let testcase = problem
            .testcases
            .first()
            .expect("aplusb should have at least one testcase");
        let expected = testcase.expected.as_deref().expect("exact testcase should have expected output");

        let languages = [
            Language::Cpp,
            Language::Rust,
            Language::Python,
            Language::PyPy,
            Language::Java,
            Language::Go,
            Language::Text,
        ];

        for language in languages {
            let work_dir = tempfile::tempdir().expect("failed to create tempdir");
            let compiled = compile(source_for(&language), &language, work_dir.path())
                .await
                .unwrap_or_else(|e| panic!("{language:?}: compile step failed: {e}"));

            assert!(
                compiled.error.is_none(),
                "{language:?}: compile error: {}",
                compiled.error.unwrap_or_default()
            );

            let run = run_in_sandbox(
                &compiled.executable,
                compiled.run_args,
                testcase.input.as_bytes(),
                sandbox_config(
                    &language,
                    problem.memory_limit_kb,
                    problem.time_limit_ms,
                ),
            )
            .await
            .unwrap_or_else(|e| panic!("{language:?}: run failed: {e}"));

            assert!(
                matches!(run.status, RunStatus::Ok),
                "{language:?}: unexpected run status: {:?}, stderr={}",
                run.status,
                String::from_utf8_lossy(&run.stderr)
            );

            let stdout = String::from_utf8_lossy(&run.stdout);
            if matches!(language, Language::Text) {
                assert_ne!(
                    stdout.trim(),
                    expected.trim(),
                    "Text should not solve aplusb by echoing stdin"
                );
            } else {
                assert_eq!(
                    stdout.trim(),
                    expected.trim(),
                    "{language:?}: unexpected stdout, stderr={}",
                    String::from_utf8_lossy(&run.stderr)
                );
            }
        }
    }
}
