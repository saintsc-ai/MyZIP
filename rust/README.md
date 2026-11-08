# MyZIP — Rust 판

Python(PySide6) 판 MyZIP 을 Rust 로 다시 쓰는 작업 공간입니다.
목표는 동작은 그대로 두고, 실행 파일을 작게(수십 MB → 수 MB) 만들고 시작을 빠르게
하는 것입니다. 탐색기 우클릭 메뉴로 자주 뜨는 프로그램이라 이 두 가지가 체감 품질을 좌우합니다.

Python 판은 옮기는 동안 그대로 유지합니다. 각 단계는 Python 판의 테스트를 기준으로 삼습니다.

## 단계

| 단계 | 내용 | 상태 |
| --- | --- | --- |
| 1 | **코어 라이브러리** `crates/myzip-core`: ZIP/TAR 읽기·쓰기, 7-Zip 을 통한 RAR/7Z 해제, 인코딩 판별, 경로 탈출 차단 | ✅ 완료 |
| 2 | **명령줄 + 셸 통합** `crates/myzip`: `--extract-here` / `--extract-to` / `--compress-zip` / `--compress-tgz` / `--test` / `--list`, `--register` / `--unregister`(HKCU 레지스트리), 여러 프로세스로 흩어진 선택을 모으는 단일 인스턴스 처리, 설정 읽기 | ✅ 완료 |
| 3 | **GUI** (egui / eframe): 메인 창(폴더 트리 + 목록), 압축·해제·진행·설정 대화상자, 암호 입력 창, 맑은 고딕 글꼴 로드, 미리보기(임시 폴더로 꺼내 열기) | 다음 |
| 4 | **배포**: exe 아이콘·버전 리소스, Inno Setup 스크립트를 Rust 빌드로 교체, CI 릴리스, Python 판 은퇴 | |

## 빌드와 테스트

Windows 에서 MSVC 툴체인(Visual Studio Build Tools 의 C++ 도구)이 필요합니다.
`liblzma` 가 C 코드를 함께 빌드하기 때문입니다.

```powershell
cd rust
cargo test                       # 전체 테스트
cargo clippy --all-targets       # 정적 검사
cargo build --release
```

RAR/7Z 테스트는 `7z.exe` 가 있어야 돌고, 없으면 스스로 건너뜁니다.
저장소 루트에서 `python tools/fetch_7zip.py` 를 실행하면 `bin/` 에 받아 둡니다.
다른 위치의 7z.exe 를 쓰려면 환경 변수 `MYZIP_7Z` 로 지정하세요.

## 셸 통합 (`crates/myzip`)

실행 파일 이름은 Python 판과 같은 `MyZIP.exe` 이고, 레지스트리 키 이름과 구조도 같습니다.
어느 판으로 설치했든 다른 판의 `--unregister` 로 지울 수 있습니다.
설정도 Python 판과 같은 자리(`HKCU\Software\MyZIP\MyZIP`)에서 읽으므로 기존 설정이 이어집니다.

| 탐색기 메뉴 | 여러 개를 골랐을 때 |
| --- | --- |
| 여기에 압축 풀기 | 아카이브마다 같은 폴더에 풉니다. 이름이 겹치면 설정(기본: 이름 바꾸기)대로 |
| 압축 풀기 (폴더 생성) | 아카이브마다 **자기 이름의 폴더**를 만들어 풉니다 (`a.zip` → `a\`, `b.tar.gz` → `b\`) |
| 무결성 검사 | 모두 검사하고 결과를 한 번에 보여 줍니다 |
| ZIP / TAR.GZ 로 압축 | 고른 것을 **하나의 아카이브**로 묶습니다 (이름은 들어 있는 폴더 이름) |

동작 방식:

- 탐색기는 고른 파일마다 프로세스를 하나씩 띄웁니다. 먼저 뜬 프로세스가 named pipe
  (`\\.\pipe\MyZIP.SingleInstance.v1.<사용자>`) 서버가 되고, 나머지는 요청을 넘기고 바로 끝납니다.
  서버는 마지막 요청이 온 뒤 0.5초를 기다렸다가 모은 것을 차례로 처리합니다.
  파일 40개를 동시에 압축하면 ZIP 하나가, 아카이브 40개를 동시에 풀면 폴더 40개가 생기는 것을 확인했습니다.
- 하나가 실패해도 나머지는 계속 처리하고, 끝에 실패한 것만 한 번에 알립니다.
- 성공하면 알림창을 띄우지 않습니다. 하나만 풀었을 때는 탐색기로 결과를 보여 줍니다(설정 `extract/open_after`).
- 압축 풀기 메뉴에도 `MultiSelectModel=Player` 를 답니다. Python 판은 이 값이 없어서
  아카이브를 16개 이상 고르면 Windows 가 '압축 풀기' 메뉴를 숨겼습니다.
- 콘솔 없는 exe 라 탐색기에서 실행해도 검은 창이 뜨지 않습니다. 터미널에서 실행하면
  부모 콘솔에 결과를 찍고, 탐색기에서 실행하면 메시지 상자로 알립니다.

아직 안 되는 것 (3단계에서): '열기', '압축 풀기...', '압축하기...' 처럼 창이 필요한 메뉴,
진행률 창과 취소 버튼, 암호 입력 창. 지금은 암호가 걸린 아카이브는 `--password` 로만 풀 수 있습니다.

### 테스트용 환경 변수

| 변수 | 효과 |
| --- | --- |
| `MYZIP_REG_ROOT` | 모든 레지스트리 경로를 이 키 아래로 옮긴다. 테스트가 실제 탐색기 설정을 건드리지 않게 한다 |
| `MYZIP_NO_SHELL` | 탐색기 창과 메시지 상자를 띄우지 않는다 (Python 판과 같음) |
| `MYZIP_NO_IPC` | 요청을 모으지 않고 혼자 처리한다 |
| `MYZIP_IPC_NAME` | 파이프 이름을 바꾼다 |
| `MYZIP_7Z` | 7z.exe 위치 |

### 직접 써 보기

Rust 판으로 탐색기 메뉴를 바꿔 끼우려면 (Python 판 등록을 덮어씁니다):

```powershell
cargo build --release
.\target\release\MyZIP.exe --register-menu   # 메뉴만. 확장자 연결은 그대로
# 되돌리기: Python 판 설치 폴더의 MyZIP.exe --register-menu
```

릴리스 exe 는 약 1.9MB 이고, 한 번 실행에 약 20ms 가 걸립니다(Python 판 약 180ms).

## 코어 구조 (`crates/myzip-core/src`)

| 파일 | Python 판 | 하는 일 |
| --- | --- | --- |
| `archive.rs` | `core/base.py` | `ArchiveReader` / `ArchiveWriter` 트레이트, 공용 해제 절차(경로 검증, 충돌 처리, 임시 파일, 시각 복원) |
| `entry.rs` | `core/base.py` | `Entry`, `Progress`, `CancelToken`, `ConflictAction` |
| `error.rs` | `core/base.py` 의 예외들 | `Error` enum (`PasswordRequired`, `Cancelled` 등) |
| `formats.rs` | `core/formats.py` | 매직 바이트 판별, `open_archive`, `writer_for` |
| `encoding.rs` | `core/encoding.py` | 파일명 인코딩 자동 판별 (UTF-8 / CP949 / CP932 / GBK / Big5 / CP1252) |
| `path_safety.rs` | `core/path_safety.py` | Zip Slip 차단, Windows 예약어·금지 문자 처리 |
| `zip_handler.rs` | `core/zip_handler.py` | ZIP (AES-256 쓰기, ZipCrypto/AES 읽기) |
| `tar_handler.rs` | `core/tar_handler.py` | TAR / TAR.GZ / TAR.BZ2 / TAR.XZ, 단일 .gz/.bz2/.xz |
| `sevenzip.rs` | `core/sevenzip.py` | 7z.exe 찾기·실행·출력 파싱 |
| `sevenzip_reader.rs` | `core/rar_handler.py` | RAR / 7Z / CAB / ARJ / LZH / ISO 리더 |

### Python 판과 달라진 점

- **반쪽짜리 파일을 코어가 막습니다.** Python 판은 UI 작업 스레드(`ui/workers.py`)가
  임시 파일을 만들었다 옮겼습니다. Rust 판은 해제·압축 모두 코어에서 `*.myzip-part`
  임시 파일에 쓴 뒤 옮기므로, CLI 로 써도 취소나 실패 시 찌꺼기가 남지 않습니다.
- **TAR 을 한 번만 훑습니다.** 압축된 TAR 은 중간으로 건너뛸 수 없어서, 여러 항목을
  꺼낼 때 스트림을 처음부터 한 번만 읽으며 필요한 항목을 만날 때마다 꺼냅니다.
- **ZIP 의 UTF-8 플래그를 직접 읽습니다.** Python 판은 `zipfile` 이 CP437 로 잘못 풀어 둔
  이름을 되돌려 원본 바이트를 복원했습니다. Rust 판은 원본 바이트와 플래그를 바로 씁니다.
- **TAR 이름의 앞쪽 `./` 만 떼어 냅니다.** Python 판의 `lstrip("./")` 는 `.hidden` 을
  `hidden` 으로 바꿔 버리는 문제가 있었습니다.
- **최상위 폴더를 벗길 때도 진행률이 100% 가 됩니다.** 벗겨 낸 폴더 항목도 완료로 셉니다.
- **무결성 검사 중 취소하면 실제로 멈춥니다.** Python 판은 `except Exception` 이 취소까지
  삼켜서 그 항목을 '손상'으로 표시하고 계속 진행했습니다.

### 호환성

Python 판으로 만든 ZIP(AES 포함)·TAR.XZ·TAR.BZ2 를 Rust 판이 읽고, Rust 판으로 만든
ZIP(AES 포함)·TAR.XZ·TGZ 를 Python 판이 읽는 것을 확인했습니다.
TAR 은 Python 판의 PAX 대신 GNU 형식으로 씁니다. 두 판과 7-Zip 모두 읽을 수 있습니다.
