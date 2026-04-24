/// seccomp によるシステムコールのホワイトリスト制限。
///
/// デフォルトアクション: `KillProcess`（許可リスト以外は即座にプロセスをKill）
/// 対象: C++ / Rust の競技プログラミング解答が必要とする最小限のシスコール
///
/// # 注意
/// - 言語ランタイムによっては追加のシスコールが必要になる場合がある。
/// - `open` / `openat` はここでは許可しているが、将来的には
///   引数フィルタで読み取り専用に制限すべき。
/// - 現状はサンドボックス適用後に `execvp` で提出プログラムを起動するため、
///   `execve` / `execveat` は許可している。

#[cfg(target_os = "linux")]
pub fn apply_filter() -> anyhow::Result<()> {
    use libseccomp::{ScmpAction, ScmpFilterContext, ScmpSyscall};

    let mut ctx = ScmpFilterContext::new_filter(ScmpAction::KillProcess)?;

    let allowed: &[&str] = &[
        // --- 入出力 ---
        "read",
        "write",
        "readv",
        "writev",
        "pread64",
        "pwrite64",
        // --- ファイルディスクリプタ ---
        "close",
        "fcntl",
        "fstat",
        "newfstatat",
        "lseek",
        // ファイルオープン（読み取りのみを想定; 将来は引数でフィルタ）
        "open",
        "openat",
        "access",
        "faccessat",
        "faccessat2",
        "getdents64",
        "readlinkat",
        // --- メモリ管理 ---
        "brk",
        "mmap",
        "munmap",
        "mprotect",
        "mremap",
        "madvise",
        // --- プロセス終了 ---
        "exit",
        "exit_group",
        // --- シグナル ---
        "rt_sigaction",
        "rt_sigprocmask",
        "rt_sigreturn",
        "sigaltstack",
        // --- プロセス起動 ---
        "execve",
        "execveat",
        // --- スレッド基盤（std / libstdc++ が使う） ---
        "futex",
        "set_tid_address",
        "set_robust_list",
        "sched_getaffinity",
        "rseq",
        "gettid",
        "getpid",
        // --- x86-64 TLS / CRT 初期化 ---
        "arch_prctl",
        // --- 時刻 ---
        "clock_gettime",
        "clock_getres",
        "gettimeofday",
        "ppoll",
        "prlimit64",
        // --- エントロピー（Rust std が使う） ---
        "getrandom",
        // --- UID/GID 問い合わせ（一部ランタイムが使う） ---
        "getuid",
        "geteuid",
        "getgid",
        "getegid",
        // --- その他 ---
        "getcwd",
        "sysinfo",
        "statx",
        "uname",
        "ioctl", // 端末検出に使われることがある
    ];

    for name in allowed {
        match ScmpSyscall::from_name(name) {
            Ok(sc) => ctx.add_rule(ScmpAction::Allow, sc)?,
            Err(_) => {
                // このカーネルに存在しないシスコール名は無視
            }
        }
    }

    ctx.load()?;
    Ok(())
}

#[cfg(not(target_os = "linux"))]
pub fn apply_filter() -> anyhow::Result<()> {
    // seccomp は Linux 専用
    Ok(())
}
