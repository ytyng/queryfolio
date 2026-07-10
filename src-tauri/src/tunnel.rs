use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use ssh2::Session;

use crate::config::SshTunnelConfig;
use crate::error::AppError;
use crate::config::expand_tilde;

/// SSH の blocking 操作 (handshake / auth 等) のタイムアウト (ミリ秒)。
const SSH_TIMEOUT_MS: u32 = 30_000;

/// 非同期ポンプループのアイドル時スリープ。
const PUMP_IDLE_SLEEP: Duration = Duration::from_millis(5);

/// SSH ローカルポートフォワードトンネル。
///
/// 127.0.0.1 の空きポートで listen し、接続を受けるたびに新しい SSH セッションを
/// 確立して direct-tcpip チャンネルで転送先へ中継する。
/// libssh2 のセッションはスレッド間の同時操作が安全でないため、
/// 転送 1 本ごとに独立したセッションを持たせる (sqlx のプールは少数接続なので
/// セッション確立のオーバーヘッドは許容範囲)。
pub struct SshTunnel {
    pub local_port: u16,
    shutdown: Arc<AtomicBool>,
}

impl Drop for SshTunnel {
    fn drop(&mut self) {
        self.shutdown.store(true, Ordering::SeqCst);
    }
}

struct ForwardTarget {
    ssh_config: SshTunnelConfig,
    target_host: String,
    target_port: u16,
}

impl SshTunnel {
    /// トンネルを開始する。認証エラーを早期に検出するため、
    /// 最初に 1 度セッション確立を試してから listener を立てる。
    pub fn start(
        ssh_config: &SshTunnelConfig,
        target_host: &str,
        target_port: u16,
    ) -> Result<Self, AppError> {
        // 認証情報の検証を兼ねた接続テスト
        establish_session(ssh_config)?;

        let listener = TcpListener::bind("127.0.0.1:0")?;
        let local_port = listener.local_addr()?.port();
        listener.set_nonblocking(true)?;

        let shutdown = Arc::new(AtomicBool::new(false));
        let target = Arc::new(ForwardTarget {
            ssh_config: ssh_config.clone(),
            target_host: target_host.to_string(),
            target_port,
        });

        let accept_shutdown = Arc::clone(&shutdown);
        std::thread::spawn(move || {
            accept_loop(listener, target, accept_shutdown);
        });

        Ok(Self {
            local_port,
            shutdown,
        })
    }
}

fn accept_loop(
    listener: TcpListener,
    target: Arc<ForwardTarget>,
    shutdown: Arc<AtomicBool>,
) {
    loop {
        if shutdown.load(Ordering::SeqCst) {
            return;
        }
        match listener.accept() {
            Ok((stream, _addr)) => {
                let target = Arc::clone(&target);
                let shutdown = Arc::clone(&shutdown);
                std::thread::spawn(move || {
                    if let Err(e) = forward_connection(stream, &target, shutdown) {
                        eprintln!("[SshTunnel] 転送エラー: {e}");
                    }
                });
            }
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                std::thread::sleep(Duration::from_millis(100));
            }
            Err(e) => {
                eprintln!("[SshTunnel] accept エラー: {e}");
                return;
            }
        }
    }
}

/// SSH セッションを確立して認証まで行う。
fn establish_session(config: &SshTunnelConfig) -> Result<Session, AppError> {
    // session.set_timeout は接続確立後にしか効かないため、
    // TCP 接続自体にも同じタイムアウトを適用する (ブラックホール化した
    // ホストで OS の TCP タイムアウトまで待たされるのを防ぐ)
    let addr = format!("{}:{}", config.host, config.port);
    let socket_addrs: Vec<std::net::SocketAddr> = std::net::ToSocketAddrs::to_socket_addrs(&addr)
        .map_err(|e| AppError::SshTunnel(format!("{addr} の名前解決に失敗: {e}")))?
        .collect();
    let mut tcp = None;
    let mut last_error = None;
    for socket_addr in &socket_addrs {
        match TcpStream::connect_timeout(
            socket_addr,
            Duration::from_millis(SSH_TIMEOUT_MS as u64),
        ) {
            Ok(stream) => {
                tcp = Some(stream);
                break;
            }
            Err(e) => last_error = Some(e),
        }
    }
    let tcp = tcp.ok_or_else(|| {
        AppError::SshTunnel(format!(
            "{addr} への接続に失敗: {}",
            last_error
                .map(|e| e.to_string())
                .unwrap_or_else(|| "アドレスを解決できませんでした".into())
        ))
    })?;

    let mut session = Session::new()
        .map_err(|e| AppError::SshTunnel(format!("セッション作成に失敗: {e}")))?;
    session.set_timeout(SSH_TIMEOUT_MS);
    session.set_tcp_stream(tcp);
    session
        .handshake()
        .map_err(|e| AppError::SshTunnel(format!("ハンドシェイクに失敗: {e}")))?;

    verify_host_key(&session, config)?;
    authenticate(&session, config)?;
    Ok(session)
}

/// ホストキーを ~/.ssh/known_hosts と照合する。
/// 不一致は MITM の可能性があるためエラー。未登録ホストは known_hosts に
/// 追記して許可する (OpenSSH の StrictHostKeyChecking=accept-new 相当)。
fn verify_host_key(session: &Session, config: &SshTunnelConfig) -> Result<(), AppError> {
    let (key, key_type) = session.host_key().ok_or_else(|| {
        AppError::SshTunnel("ホストキーを取得できませんでした".into())
    })?;

    let mut known_hosts = session
        .known_hosts()
        .map_err(|e| AppError::SshTunnel(format!("known_hosts の初期化に失敗: {e}")))?;

    let known_hosts_path = dirs::home_dir()
        .ok_or_else(|| AppError::SshTunnel("ホームディレクトリを特定できません".into()))?
        .join(".ssh")
        .join("known_hosts");

    if known_hosts_path.exists() {
        known_hosts
            .read_file(&known_hosts_path, ssh2::KnownHostFileKind::OpenSSH)
            .map_err(|e| {
                AppError::SshTunnel(format!(
                    "known_hosts の読み込みに失敗 ({}): {e}",
                    known_hosts_path.display()
                ))
            })?;
    }

    match known_hosts.check_port(&config.host, config.port, key) {
        ssh2::CheckResult::Match => Ok(()),
        ssh2::CheckResult::Mismatch => Err(AppError::SshTunnel(format!(
            "{} のホストキーが known_hosts と一致しません。中間者攻撃の可能性があるため接続を中止しました。\
             ホストキーが正しく変更された場合は known_hosts の該当行を削除してください",
            config.host
        ))),
        ssh2::CheckResult::NotFound => {
            // known_hosts のエントリ形式: 標準ポートはホスト名のみ、
            // 非標準ポートは [host]:port
            let entry_host = if config.port == 22 {
                config.host.clone()
            } else {
                format!("[{}]:{}", config.host, config.port)
            };
            known_hosts
                .add(&entry_host, key, "added by queryfolio", key_type.into())
                .map_err(|e| {
                    AppError::SshTunnel(format!("known_hosts への追加に失敗: {e}"))
                })?;
            if let Some(parent) = known_hosts_path.parent() {
                std::fs::create_dir_all(parent)?;
            }
            known_hosts
                .write_file(&known_hosts_path, ssh2::KnownHostFileKind::OpenSSH)
                .map_err(|e| {
                    AppError::SshTunnel(format!(
                        "known_hosts の書き込みに失敗 ({}): {e}",
                        known_hosts_path.display()
                    ))
                })?;
            Ok(())
        }
        ssh2::CheckResult::Failure => Err(AppError::SshTunnel(
            "ホストキーの検証処理に失敗しました".into(),
        )),
    }
}

fn authenticate(session: &Session, config: &SshTunnelConfig) -> Result<(), AppError> {
    if let Some(key_path) = &config.private_key_path {
        let key_path: PathBuf = expand_tilde(key_path);
        session
            .userauth_pubkey_file(
                &config.user,
                None,
                &key_path,
                config.private_key_passphrase.as_deref(),
            )
            .map_err(|e| {
                AppError::SshTunnel(format!(
                    "秘密鍵認証に失敗 ({}): {e}",
                    key_path.display()
                ))
            })?;
        return Ok(());
    }

    if let Some(password) = &config.password {
        session
            .userauth_password(&config.user, password)
            .map_err(|e| AppError::SshTunnel(format!("パスワード認証に失敗: {e}")))?;
        return Ok(());
    }

    // password も private_key_path も無い場合は ssh-agent を試す
    session
        .userauth_agent(&config.user)
        .map_err(|e| AppError::SshTunnel(format!("ssh-agent 認証に失敗: {e}")))?;
    Ok(())
}

/// ローカル TCP 接続 1 本を SSH direct-tcpip チャンネルへ中継する。
///
/// libssh2 は同一セッションへの並行操作が安全でないため、
/// セッションを non-blocking にして単一スレッドで双方向にポンプする。
fn forward_connection(
    tcp: TcpStream,
    target: &ForwardTarget,
    shutdown: Arc<AtomicBool>,
) -> Result<(), AppError> {
    let session = establish_session(&target.ssh_config)?;
    let mut channel = session
        .channel_direct_tcpip(&target.target_host, target.target_port, None)
        .map_err(|e| {
            AppError::SshTunnel(format!(
                "direct-tcpip チャンネルの作成に失敗 ({}:{}): {e}",
                target.target_host, target.target_port
            ))
        })?;

    tcp.set_nonblocking(true)?;
    session.set_blocking(false);

    let mut tcp = tcp;
    let mut buf = [0u8; 16 * 1024];
    let mut tcp_eof = false;

    loop {
        if shutdown.load(Ordering::SeqCst) {
            break;
        }
        let mut idle = true;

        if !tcp_eof {
            match tcp.read(&mut buf) {
                Ok(0) => {
                    tcp_eof = true;
                    let _ = write_all_nonblocking_channel_eof(&mut channel);
                }
                Ok(n) => {
                    idle = false;
                    write_all_nonblocking(
                        |data| channel.write(data),
                        &buf[..n],
                    )?;
                }
                Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {}
                Err(e) => return Err(e.into()),
            }
        }

        match channel.read(&mut buf) {
            Ok(0) => {
                // チャンネル側 EOF: 中継終了
                break;
            }
            Ok(n) => {
                idle = false;
                write_all_nonblocking(|data| tcp.write(data), &buf[..n])?;
            }
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {}
            Err(e) => return Err(e.into()),
        }

        if tcp_eof && channel.eof() {
            break;
        }
        if idle {
            std::thread::sleep(PUMP_IDLE_SLEEP);
        }
    }
    Ok(())
}

/// non-blocking な writer に対して全バイト書き込む。WouldBlock はリトライする。
fn write_all_nonblocking<W>(mut write: W, data: &[u8]) -> Result<(), std::io::Error>
where
    W: FnMut(&[u8]) -> Result<usize, std::io::Error>,
{
    let mut offset = 0;
    while offset < data.len() {
        match write(&data[offset..]) {
            Ok(0) => {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::WriteZero,
                    "書き込み先がクローズされました",
                ));
            }
            Ok(n) => offset += n,
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                std::thread::sleep(PUMP_IDLE_SLEEP);
            }
            Err(e) => return Err(e),
        }
    }
    Ok(())
}

/// non-blocking チャンネルへの EOF 送信。WouldBlock はリトライする。
fn write_all_nonblocking_channel_eof(
    channel: &mut ssh2::Channel,
) -> Result<(), std::io::Error> {
    loop {
        match channel.send_eof() {
            Ok(()) => return Ok(()),
            Err(e) => {
                let io_err: std::io::Error = e.into();
                if io_err.kind() == std::io::ErrorKind::WouldBlock {
                    std::thread::sleep(PUMP_IDLE_SLEEP);
                    continue;
                }
                return Err(io_err);
            }
        }
    }
}
