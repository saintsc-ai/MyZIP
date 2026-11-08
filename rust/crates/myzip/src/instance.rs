//! 단일 인스턴스: 흩어져 들어온 요청을 한 프로세스로 모은다.
//!
//! 탐색기에서 파일 30개를 골라 메뉴를 누르면 Windows 는 MyZIP 을 30번 띄운다.
//! 그대로 두면 ZIP 이 30개 생기고, 압축 풀기 30개가 동시에 디스크를 긁는다.
//!
//! 그래서 먼저 뜬 프로세스가 named pipe 서버가 된다. 나중에 뜬 프로세스는
//! 자기 요청을 서버에 넘기고 바로 끝난다. 서버는 요청이 잠잠해질 때까지
//! (마지막 도착 후 0.5초) 기다렸다가 접수를 마감하고, 모은 것을 창 하나에서
//! 차례로 처리한다. 마감 뒤에 들어온 요청은 새 서버가 받아 자기 창에서 처리한다.
//!
//! 넘기는 데 실패하면(서버가 막 끝나는 중이었다든지) 스스로 처리한다.
//! 요청이 사라지는 일은 없어야 한다.

use std::fs::{File, OpenOptions};
use std::io::{self, Read, Write};
use std::mem::ManuallyDrop;
use std::os::windows::io::FromRawHandle;
use std::path::PathBuf;
use std::sync::mpsc::{self, Sender};
use std::thread;
use std::time::Duration;

use serde_json::{Value, json};
use windows_sys::Win32::Foundation::{
    CloseHandle, ERROR_PIPE_BUSY, ERROR_PIPE_CONNECTED, GetLastError, HANDLE, INVALID_HANDLE_VALUE,
};
use windows_sys::Win32::Storage::FileSystem::{
    FILE_FLAG_FIRST_PIPE_INSTANCE, FlushFileBuffers, PIPE_ACCESS_DUPLEX,
};
use windows_sys::Win32::System::Pipes::{
    ConnectNamedPipe, CreateNamedPipeW, DisconnectNamedPipe, PIPE_READMODE_BYTE,
    PIPE_REJECT_REMOTE_CLIENTS, PIPE_TYPE_BYTE, PIPE_WAIT, WaitNamedPipeW,
};

use crate::cli::{Mode, Request};

/// 파이프 이름을 바꾸는 환경 변수 (테스트가 서로 엉키지 않게).
pub const ENV_NAME: &str = "MYZIP_IPC_NAME";
/// 모으지 않고 혼자 처리하게 하는 환경 변수.
pub const ENV_NO_IPC: &str = "MYZIP_NO_IPC";

/// 흩어져 들어온 요청을 기다리는 시간. 마지막 도착부터 센다.
const COLLECT_DELAY: Duration = Duration::from_millis(500);
const STOP: &str = "__stop__";
const MAX_MESSAGE: u32 = 16 << 20;

fn pipe_name() -> String {
    if let Ok(name) = std::env::var(ENV_NAME) {
        return format!(r"\\.\pipe\{name}");
    }
    // 같은 PC 의 다른 사용자(원격 데스크톱)와 섞이지 않게 사용자 이름을 붙인다.
    let user = std::env::var("USERNAME").unwrap_or_default();
    format!(r"\\.\pipe\MyZIP.SingleInstance.v1.{user}")
}

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(Some(0)).collect()
}

// ---------------------------------------------------------------- 메시지

enum Message {
    Request(Request),
    Stop,
}

/// 요청을 JSON 으로. 유니코드가 아닌 경로가 섞이면 None (그러면 혼자 처리한다).
fn encode(request: &Request) -> Option<Vec<u8>> {
    let paths: Option<Vec<&str>> = request.paths.iter().map(|p| p.to_str()).collect();
    let value = json!({
        "mode": request.mode.as_str(),
        "paths": paths?,
        "password": request.password,
    });
    Some(value.to_string().into_bytes())
}

fn stop_message() -> Vec<u8> {
    json!({ "mode": STOP }).to_string().into_bytes()
}

fn decode(bytes: &[u8]) -> Option<Message> {
    let value: Value = serde_json::from_slice(bytes).ok()?;
    let mode = value.get("mode")?.as_str()?;
    if mode == STOP {
        return Some(Message::Stop);
    }
    let paths = value
        .get("paths")?
        .as_array()?
        .iter()
        .map(|p| p.as_str().map(PathBuf::from))
        .collect::<Option<Vec<_>>>()?;
    let password = value
        .get("password")
        .and_then(Value::as_str)
        .map(str::to_owned);
    Some(Message::Request(Request {
        mode: Mode::parse(mode)?,
        paths,
        password,
    }))
}

fn read_frame(reader: &mut impl Read) -> io::Result<Vec<u8>> {
    let mut len = [0u8; 4];
    reader.read_exact(&mut len)?;
    let len = u32::from_le_bytes(len);
    if len > MAX_MESSAGE {
        return Err(io::Error::other("메시지가 너무 큽니다"));
    }
    let mut body = vec![0u8; len as usize];
    reader.read_exact(&mut body)?;
    Ok(body)
}

// ---------------------------------------------------------------- 클라이언트

/// 떠 있는 서버에 메시지를 넘긴다. 서버가 받았다고 답해야 성공이다.
fn send(name: &str, payload: &[u8]) -> io::Result<()> {
    for _ in 0..20 {
        match OpenOptions::new().read(true).write(true).open(name) {
            Ok(mut pipe) => {
                pipe.write_all(&(payload.len() as u32).to_le_bytes())?;
                pipe.write_all(payload)?;
                pipe.flush()?;
                let mut ack = [0u8; 1];
                pipe.read_exact(&mut ack)?;
                return if &ack == b"K" {
                    Ok(())
                } else {
                    Err(io::Error::other("잘못된 응답"))
                };
            }
            // 서버가 다른 요청을 받는 중이다. 잠깐 기다렸다 다시.
            Err(e) if e.raw_os_error() == Some(ERROR_PIPE_BUSY as i32) => {
                let name = wide(name);
                // SAFETY: name 은 null 로 끝난다.
                unsafe { WaitNamedPipeW(name.as_ptr(), 1000) };
            }
            Err(e) => return Err(e),
        }
    }
    Err(io::Error::other("서버가 계속 바쁩니다"))
}

// ---------------------------------------------------------------- 서버

/// 파이프 핸들. 스레드로 넘기기 위해 감싼다.
struct Server(HANDLE);

// SAFETY: 핸들은 리스너 스레드 하나만 쓴다.
unsafe impl Send for Server {}

/// 첫 번째 인스턴스 자리를 잡는다. 이미 다른 프로세스가 잡았으면 None.
fn claim(name: &str) -> Option<Server> {
    let wide_name = wide(name);
    // SAFETY: 이름은 null 로 끝나고, 보안 속성은 기본값(null)을 쓴다.
    // FILE_FLAG_FIRST_PIPE_INSTANCE 덕분에 둘이 동시에 서버가 되는 일은 없다.
    let handle = unsafe {
        CreateNamedPipeW(
            wide_name.as_ptr(),
            PIPE_ACCESS_DUPLEX | FILE_FLAG_FIRST_PIPE_INSTANCE,
            PIPE_TYPE_BYTE | PIPE_READMODE_BYTE | PIPE_WAIT | PIPE_REJECT_REMOTE_CLIENTS,
            1,
            4096,
            64 * 1024,
            0,
            std::ptr::null(),
        )
    };
    (handle != INVALID_HANDLE_VALUE).then_some(Server(handle))
}

/// 요청을 하나씩 받아 채널로 넘긴다. 정지 메시지를 받으면 끝난다.
fn listen(server: Server, tx: Sender<Request>) {
    let handle = server.0;
    // SAFETY: 핸들은 유효한 파이프 핸들이다. File 이 닫지 않도록 ManuallyDrop 으로 둔다.
    let pipe = ManuallyDrop::new(unsafe { File::from_raw_handle(handle) });
    loop {
        // SAFETY: 동기 파이프이므로 OVERLAPPED 없이 기다린다.
        let connected = unsafe { ConnectNamedPipe(handle, std::ptr::null_mut()) } != 0
            || unsafe { GetLastError() } == ERROR_PIPE_CONNECTED;
        let mut stop = false;
        let message = connected
            .then(|| read_frame(&mut &*pipe).ok().and_then(|b| decode(&b)))
            .flatten();
        if let Some(message) = message {
            match message {
                Message::Request(request) => {
                    let _ = tx.send(request);
                }
                Message::Stop => stop = true,
            }
            let _ = (&*pipe).write_all(b"K");
            // SAFETY: 유효한 핸들. 응답이 클라이언트에 닿을 때까지 기다린다.
            unsafe { FlushFileBuffers(handle) };
        }
        // SAFETY: 유효한 핸들. 다음 클라이언트를 받을 수 있게 연결을 끊는다.
        unsafe { DisconnectNamedPipe(handle) };
        if stop {
            break;
        }
    }
    // SAFETY: 이 핸들을 쓰는 곳은 더 없다.
    unsafe { CloseHandle(handle) };
}

/// 요청을 모은다. 잠잠해지면(마지막 도착 후 0.5초) 접수를 마감하고 모은 것을 돌려준다.
///
/// 마감한 뒤에 들어오는 요청은 새 프로세스가 서버가 되어 따로 처리한다.
/// 그래야 '압축 풀기' 창과 '압축하기' 창처럼 서로 다른 일이 한 창에 섞이지 않는다.
fn collect(server: Server, name: &str, first: Request) -> Vec<Request> {
    let (tx, rx) = mpsc::channel();
    let listener = thread::spawn(move || listen(server, tx));

    let mut queue = vec![first];
    while let Ok(request) = rx.recv_timeout(COLLECT_DELAY) {
        queue.push(request);
    }

    // 더 받지 않는다. 마감 직전에 끼어든 요청도 놓치지 않는다.
    // 정지 메시지를 못 보냈으면 리스너를 기다리지 않는다 (프로세스가 끝나면 함께 끝난다).
    if send(name, &stop_message()).is_ok() {
        let _ = listener.join();
    }
    queue.extend(rx.try_iter());
    queue
}

/// 흩어진 요청을 모은다.
///
/// * `Some(요청들)`: 이 프로세스가 처리해야 한다 (첫 번째 인스턴스였거나, 넘기지 못했다).
/// * `None`: 이미 떠 있는 인스턴스에 넘겼다. 이 프로세스는 할 일이 없다.
pub fn gather(request: Request) -> Option<Vec<Request>> {
    if std::env::var_os(ENV_NO_IPC).is_some() {
        return Some(vec![request]);
    }
    let Some(payload) = encode(&request) else {
        return Some(vec![request]);
    };

    let name = pipe_name();
    for _ in 0..3 {
        if let Some(server) = claim(&name) {
            return Some(collect(server, &name, request));
        }
        if send(&name, &payload).is_ok() {
            return None; // 넘겼다. 결과는 그쪽이 알린다.
        }
        // 서버가 막 마감하는 중이었다. 다시 자리를 잡아 본다.
        thread::sleep(Duration::from_millis(50));
    }
    Some(vec![request])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn message_roundtrip() {
        let request = Request {
            mode: Mode::ExtractTo,
            paths: vec![PathBuf::from(r"D:\자료\a.zip")],
            password: Some("비밀".into()),
        };
        let Some(Message::Request(back)) = decode(&encode(&request).unwrap()) else {
            panic!()
        };
        assert_eq!(back, request);
        assert!(matches!(decode(&stop_message()), Some(Message::Stop)));
        assert!(decode(b"{\"mode\":\"bogus\",\"paths\":[]}").is_none());
    }
}
