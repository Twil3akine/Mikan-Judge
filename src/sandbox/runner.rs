/// fork + exec によるサンドボックス実行。
///
/// # 設計上の注意
/// - `fork()` 後の子プロセスは `execve` まで async-signal-safe な操作のみ行う。
/// - tokio の `spawn_blocking` スレッドから呼ぶことで、tokio ワーカースレッドの
///   ロック状態を気にせず fork できる（それでも完全に安全ではないが、
///   競合プログラミングジャッジの実用範囲では許容される）。
use std::ffi::CString;
#[cfg(target_os = "linux")]
use std::fs;
use std::os::unix::ffi::OsStrExt;
use std::os::unix::io::{IntoRawFd, RawFd};
use std::path::Path;
#[cfg(target_os = "linux")]
use std::path::PathBuf;
#[cfg(target_os = "linux")]
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use nix::sys::signal::Signal;
use nix::sys::wait::WaitStatus;
use nix::unistd::{ForkResult, Pid, fork};

use super::{RunResult, RunStatus, SandboxConfig};

#[cfg(target_os = "linux")]
static CGROUP_SEQ: AtomicU64 = AtomicU64::new(0);

pub fn run_sandboxed_blocking(
    executable: &Path,
    run_args: &[String],
    stdin_data: &[u8],
    config: &SandboxConfig,
) -> Result<RunResult> {
    // fork の前にパイプを作成
    let (stdin_r, stdin_w) = make_pipe()?;
    let (stdout_r, stdout_w) = make_pipe()?;
    let (stderr_r, stderr_w) = make_pipe()?;
    #[cfg(target_os = "linux")]
    let cgroup = LinuxCgroup::new();
    #[cfg(not(target_os = "linux"))]
    let cgroup: Option<()> = None;

    let start = Instant::now();

    // SAFETY: fork 後の子プロセスは execve のみ行い、
    //         Rust のランタイムやヒープを触らない。
    match unsafe { fork() }.context("fork(2) failed")? {
        ForkResult::Child => child_exec(
            stdin_r, stdin_w, stdout_r, stdout_w, stderr_r, stderr_w, executable, run_args, config,
        ),
        ForkResult::Parent { child } => parent_collect(
            child, stdin_r, stdin_w, stdout_r, stdout_w, stderr_r, stderr_w, stdin_data, config,
            start, cgroup,
        ),
    }
}

// ---- ユーティリティ ----

fn make_pipe() -> Result<(RawFd, RawFd)> {
    let (r, w) = nix::unistd::pipe().context("pipe(2) failed")?;
    Ok((r.into_raw_fd(), w.into_raw_fd()))
}

// ---- 子プロセス側 ----

/// 子プロセスでサンドボックスを設定してから `execvp` する。
/// この関数は絶対に返らない（`-> !`）。
fn child_exec(
    stdin_r: RawFd,
    stdin_w: RawFd,
    stdout_r: RawFd,
    stdout_w: RawFd,
    stderr_r: RawFd,
    stderr_w: RawFd,
    executable: &Path,
    run_args: &[String],
    config: &SandboxConfig,
) -> ! {
    // stdin/stdout/stderr を再配線
    unsafe {
        libc::dup2(stdin_r, libc::STDIN_FILENO);
        libc::dup2(stdout_w, libc::STDOUT_FILENO);
        libc::dup2(stderr_w, libc::STDERR_FILENO);

        // 不要なパイプ端をすべて閉じる
        libc::close(stdin_r);
        libc::close(stdin_w);
        libc::close(stdout_r);
        libc::close(stdout_w);
        libc::close(stderr_r);
        libc::close(stderr_w);
    }

    // ---- rlimit: リソース制限 ----

    // CPU 時間（秒）: TLE 時に SIGXCPU を送る
    let cpu_secs = config.time_limit.as_secs().max(1) + 1;
    set_rlimit(libc::RLIMIT_CPU as _, cpu_secs, cpu_secs + 1);

    // 仮想メモリ上限: None のときは制限なし（Python 等インタプリタは起動時に大量の仮想空間を使うため）
    if let Some(mem) = config.vm_limit_bytes {
        set_rlimit(libc::RLIMIT_AS as _, mem, mem);
    }

    // スタックサイズ: 64 MiB
    let stack: u64 = 64 * 1024 * 1024;
    set_rlimit(libc::RLIMIT_STACK as _, stack, stack);

    // ファイル書き込みサイズ: 16 MiB（無限ループでディスク埋め対策）
    let fsize: u64 = 16 * 1024 * 1024;
    set_rlimit(libc::RLIMIT_FSIZE as _, fsize, fsize);

    // プロセス数制限。Go / Java ランタイムは内部スレッドを使うため緩和する。
    if let Some(nproc) = config.nproc_limit {
        set_rlimit(libc::RLIMIT_NPROC as _, nproc, nproc);
    }

    // ---- 名前空間分離 ----

    #[cfg(target_os = "linux")]
    {
        use nix::sched::{CloneFlags, unshare};
        // ネットワーク名前空間を分離して外部通信を遮断
        let _ = unshare(CloneFlags::CLONE_NEWNET);
        // CLONE_NEWPID は /proc の再マウントが必要なので後回し
    }

    // ---- seccomp: システムコール制限 ----
    // non-linux では apply_filter() が Ok(()) を返すだけなので常に呼んでよい

    if config.enable_seccomp {
        if let Err(e) = super::seccomp::apply_filter() {
            let msg = format!("seccomp setup failed: {e}\n");
            unsafe {
                libc::write(
                    libc::STDERR_FILENO,
                    msg.as_ptr() as *const libc::c_void,
                    msg.len(),
                );
                libc::_exit(1);
            }
        }
    }

    // ---- execvp ----
    // PATH を使って解決するので、インタプリタ名（"python3" 等）もそのまま渡せる。

    let path = match CString::new(executable.as_os_str().as_bytes()) {
        Ok(p) => p,
        Err(_) => unsafe { libc::_exit(1) },
    };

    let mut argv: Vec<CString> = vec![path.clone()];
    for a in run_args {
        match CString::new(a.as_bytes()) {
            Ok(s) => argv.push(s),
            Err(_) => unsafe { libc::_exit(1) },
        }
    }

    let _ = nix::unistd::execvp(&path, &argv);

    // execvp が返った = 失敗
    unsafe { libc::_exit(1) }
}

// ---- 親プロセス側 ----

fn parent_collect(
    child: Pid,
    stdin_r: RawFd,
    stdin_w: RawFd,
    stdout_r: RawFd,
    stdout_w: RawFd,
    stderr_r: RawFd,
    stderr_w: RawFd,
    stdin_data: &[u8],
    config: &SandboxConfig,
    start: Instant,
    #[cfg(target_os = "linux")] cgroup: Option<LinuxCgroup>,
    #[cfg(not(target_os = "linux"))] _cgroup: Option<()>,
) -> Result<RunResult> {
    // 子プロセス側のパイプ端を親では閉じる
    unsafe {
        libc::close(stdin_r);
        libc::close(stdout_w);
        libc::close(stderr_w);
    }

    #[cfg(target_os = "linux")]
    if let Some(ref cgroup) = cgroup {
        let _ = cgroup.add_process(child);
    }

    // stdin を別スレッドで書き込む（パイプバッファを詰まらせないため）
    {
        let data = stdin_data.to_owned();
        std::thread::spawn(move || {
            let mut offset = 0;
            while offset < data.len() {
                let n = unsafe {
                    libc::write(
                        stdin_w,
                        data[offset..].as_ptr() as *const libc::c_void,
                        data.len() - offset,
                    )
                };
                if n <= 0 {
                    break;
                }
                offset += n as usize;
            }
            unsafe { libc::close(stdin_w) };
        });
    }

    // stdout / stderr を別スレッドで読み込む（パイプデッドロック防止）
    let max_out = config.max_output_bytes;
    let stdout_handle = std::thread::spawn(move || drain_fd(stdout_r, max_out));
    let stderr_handle = std::thread::spawn(move || drain_fd(stderr_r, 65_536));

    // wait4 でポーリング。
    // waitpid + getrusage(RUSAGE_CHILDREN) と違い、wait4 は「この特定の子プロセス」の
    // リソース使用量を直接返すため、他ワーカーとの混入や累積問題が起きない。
    let deadline = start + config.time_limit + Duration::from_millis(100);
    let mut final_status: Option<WaitStatus> = None;
    let mut child_rusage: libc::rusage = unsafe { std::mem::zeroed() };
    let mut killed = false;

    loop {
        let mut wstatus: libc::c_int = 0;
        let mut ru: libc::rusage = unsafe { std::mem::zeroed() };
        let ret = unsafe { libc::wait4(child.as_raw(), &mut wstatus, libc::WNOHANG, &mut ru) };

        if ret == child.as_raw() {
            child_rusage = ru;
            final_status = Some(parse_wait_status(child, wstatus));
            break;
        } else if ret == 0 {
            if Instant::now() >= deadline {
                let _ = nix::sys::signal::kill(child, Signal::SIGKILL);
                killed = true;
                // ブロッキングで回収
                let ret2 =
                    unsafe { libc::wait4(child.as_raw(), &mut wstatus, 0, &mut child_rusage) };
                if ret2 == child.as_raw() {
                    final_status = Some(parse_wait_status(child, wstatus));
                }
                break;
            }
            std::thread::sleep(Duration::from_millis(5));
        } else {
            break; // エラー
        }
    }

    let time_used = start.elapsed();

    #[cfg(target_os = "linux")]
    let memory_used_bytes = cgroup
        .as_ref()
        .and_then(LinuxCgroup::memory_peak_bytes)
        .unwrap_or_else(|| (child_rusage.ru_maxrss as u64).saturating_mul(1024));
    #[cfg(not(target_os = "linux"))]
    let memory_used_bytes = child_rusage.ru_maxrss as u64;

    // ここで join することでパイプが完全に読み切られるのを待つ
    let stdout = stdout_handle.join().unwrap_or_default();
    let stderr = stderr_handle.join().unwrap_or_default();

    let (exit_code, status) = determine_status(killed, time_used, config.time_limit, final_status);

    Ok(RunResult {
        stdout,
        stderr,
        exit_code,
        wall_time_used: time_used,
        memory_used_bytes,
        status,
    })
}

#[cfg(target_os = "linux")]
struct LinuxCgroup {
    dir: PathBuf,
}

#[cfg(target_os = "linux")]
impl LinuxCgroup {
    fn new() -> Option<Self> {
        let base = Path::new("/sys/fs/cgroup/mikan-judge");
        ensure_cgroup_dir(base).ok()?;

        let run_dir = base.join(format!(
            "run-{}-{}",
            std::process::id(),
            CGROUP_SEQ.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&run_dir).ok()?;
        Some(Self { dir: run_dir })
    }

    fn add_process(&self, pid: Pid) -> Result<()> {
        fs::write(self.dir.join("cgroup.procs"), format!("{}\n", pid.as_raw()))
            .context("failed to move child process into cgroup")
    }

    fn memory_peak_bytes(&self) -> Option<u64> {
        let raw = fs::read_to_string(self.dir.join("memory.peak")).ok()?;
        raw.trim().parse::<u64>().ok()
    }
}

#[cfg(target_os = "linux")]
impl Drop for LinuxCgroup {
    fn drop(&mut self) {
        let _ = fs::remove_dir(&self.dir);
    }
}

#[cfg(target_os = "linux")]
fn ensure_cgroup_dir(dir: &Path) -> Result<()> {
    fs::create_dir_all(dir).context("failed to create cgroup directory")?;
    let controllers = fs::read_to_string(dir.join("cgroup.controllers")).unwrap_or_default();
    if controllers.split_whitespace().any(|controller| controller == "memory") {
        let _ = fs::write(dir.join("cgroup.subtree_control"), "+memory\n");
    }
    Ok(())
}

fn determine_status(
    killed: bool,
    time_used: Duration,
    time_limit: Duration,
    final_status: Option<WaitStatus>,
) -> (Option<i32>, RunStatus) {
    if killed || time_used > time_limit + Duration::from_millis(50) {
        return (None, RunStatus::TimeLimitExceeded);
    }

    match final_status {
        Some(WaitStatus::Exited(_, code)) => {
            if code == 0 {
                (Some(code), RunStatus::Ok)
            } else {
                (Some(code), RunStatus::RuntimeError)
            }
        }
        Some(WaitStatus::Signaled(_, sig, _)) => {
            if sig == Signal::SIGXCPU {
                // RLIMIT_CPU 超過 → TLE
                (None, RunStatus::TimeLimitExceeded)
            } else {
                (None, RunStatus::Killed)
            }
        }
        _ => (None, RunStatus::RuntimeError),
    }
}

#[cfg(target_os = "linux")]
type RlimitResource = libc::__rlimit_resource_t;

#[cfg(not(target_os = "linux"))]
type RlimitResource = libc::c_int;

/// libc::setrlimit ラッパー（nix の Rlim 型に依存しない）
fn set_rlimit(resource: RlimitResource, soft: u64, hard: u64) {
    let limit = libc::rlimit {
        rlim_cur: soft as libc::rlim_t,
        rlim_max: hard as libc::rlim_t,
    };
    unsafe { libc::setrlimit(resource, &limit) };
}

/// wait4 から得た生ステータスを WaitStatus に変換する。
fn parse_wait_status(pid: Pid, raw: libc::c_int) -> WaitStatus {
    if libc::WIFEXITED(raw) {
        WaitStatus::Exited(pid, libc::WEXITSTATUS(raw))
    } else if libc::WIFSIGNALED(raw) {
        let sig = Signal::try_from(libc::WTERMSIG(raw)).unwrap_or(Signal::SIGKILL);
        WaitStatus::Signaled(pid, sig, false)
    } else {
        WaitStatus::StillAlive
    }
}

/// fd からデータを読み切って Vec<u8> で返す。max_bytes を超えた分は捨てる。
fn drain_fd(fd: RawFd, max_bytes: usize) -> Vec<u8> {
    let mut buf = vec![0u8; 4096];
    let mut out = Vec::new();
    loop {
        let n = unsafe { libc::read(fd, buf.as_mut_ptr() as *mut libc::c_void, buf.len()) };
        if n <= 0 {
            break;
        }
        let n = n as usize;
        let remaining = max_bytes.saturating_sub(out.len());
        if remaining > 0 {
            out.extend_from_slice(&buf[..n.min(remaining)]);
        }
    }
    unsafe { libc::close(fd) };
    out
}
