# MyZIP — Rust 판

MyZIP 2.0 부터는 이 Rust 판이 본체입니다. 1.x 는 Python(PySide6) 판이었고,
그 소스는 git 기록(`v1.0.1` 까지)에 남아 있습니다. 아래 표와 설명에서 "Python 판" 은 그것을 말합니다.

다시 쓴 목적은 동작은 그대로 두고, 배포판을 작게(설치 파일 24.1MB → 5.3MB) 만들고 시작을 빠르게
하는 것이었습니다. 탐색기 우클릭 메뉴로 자주 뜨는 프로그램이라 이 두 가지가 체감 품질을 좌우합니다.
각 단계는 Python 판의 테스트를 옮겨 기준으로 삼았습니다.

## 단계

| 단계 | 내용 | 상태 |
| --- | --- | --- |
| 1 | **코어 라이브러리** `crates/myzip-core`: ZIP/TAR 읽기·쓰기, 7-Zip 을 통한 RAR/7Z 해제, 인코딩 판별, 경로 탈출 차단 | ✅ 완료 |
| 2 | **명령줄 + 셸 통합** `crates/myzip`: `--extract-here` / `--extract-to` / `--compress-zip` / `--compress-tgz` / `--test` / `--list`, `--register` / `--unregister`(HKCU 레지스트리), 여러 프로세스로 흩어진 선택을 모으는 단일 인스턴스 처리, 설정 읽기 | ✅ 완료 |
| 3 | **GUI** (egui / eframe): 메인 창(폴더 트리 + 목록), 압축·해제·진행·설정 대화상자, 암호 입력 창, 맑은 고딕 글꼴 로드, 미리보기(임시 폴더로 꺼내 열기) | ✅ 완료 |
| 4 | **배포**: exe 아이콘·버전 리소스, Inno Setup 스크립트를 Rust 빌드로 교체, CI 릴리스, Python 판 은퇴 | ✅ 완료 |

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

창을 띄울지는 이렇게 정합니다.

- 탐색기 메뉴나 터미널에서 실행하면 작업 창(진행률, 취소, 암호 입력)을 띄웁니다.
- 출력을 파이프나 파일로 돌려 실행하면(스크립트) 창 없이 처리하고 결과를 글자로 돌려줍니다.
- 그래픽을 쓸 수 없는 환경(OpenGL 이 없는 원격 세션 등)이면 창 없이 처리하고 메시지 상자로 알립니다.

### 테스트용 환경 변수

| 변수 | 효과 |
| --- | --- |
| `MYZIP_REG_ROOT` | 모든 레지스트리 경로를 이 키 아래로 옮긴다. 테스트가 실제 탐색기 설정을 건드리지 않게 한다 |
| `MYZIP_NO_SHELL` | 탐색기 창과 메시지 상자를 띄우지 않는다 (Python 판과 같음) |
| `MYZIP_NO_IPC` | 요청을 모으지 않고 혼자 처리한다 |
| `MYZIP_IPC_NAME` | 파이프 이름을 바꾼다 |
| `MYZIP_7Z` | 7z.exe 위치 |

### 직접 써 보기

개발 중인 빌드로 탐색기 메뉴를 바꿔 끼우려면 (설치된 MyZIP 의 등록을 덮어씁니다):

```powershell
cargo build --release
.\target\release\MyZIP.exe --register-menu   # 메뉴만. 확장자 연결은 그대로
# 되돌리기: 설치 폴더의 MyZIP.exe --register-menu
```

릴리스 exe 는 약 7.5MB(GUI 포함)이고, 창이 뜨기까지 약 0.2초가 걸립니다.

## 배포 (4단계)

`python build.py --release` (저장소 루트)가 `cargo build --release` → 배포 폴더 → 동작 확인 →
설치 파일 → 포터블 ZIP → 체크섬 순서로 만듭니다.

| | 1.x (Python) | 2.0 (Rust) |
| --- | --- | --- |
| 배포 폴더 | `MyZIP.exe` + `_internal/` (Qt, Python 런타임 등 200개 넘는 파일, 약 85MB) | `MyZIP.exe` + `bin/7z.exe`, `bin/7z.dll` + 라이선스 (5개 파일, 9.8MB) |
| 설치 파일 | 24.1MB | 5.3MB |
| 포터블 ZIP | 33.0MB | 4.8MB |

- **리소스** (`crates/myzip/build.rs`): 아이콘 7개, 버전 정보(파일 속성의 "자세히" 탭),
  매니페스트(공용 컨트롤 6, PerMonitorV2 DPI, 긴 경로, asInvoker)를 exe 에 넣습니다.
  `rc.exe` 는 Windows SDK 에서 `embed-resource` 가 찾아 씁니다.
- **아이콘**: 레지스트리에는 `MyZIP.exe,-<리소스 ID>` 로 적습니다. 이제 `.ico` 파일을 배포하지 않습니다.
  ID 표는 build.rs 한 곳에서 정하고, 빌드 때 `icon_ids.rs` 를 만들어 `registry.rs` 가 읽습니다.
- **버전**은 `rust/Cargo.toml` 의 `[workspace.package] version` 한 곳에서만 정합니다.
  exe 버전 정보, `--version`, 설치 파일(`/DAppVersion=`), 릴리스 태그 검사가 모두 여기서 읽습니다.
- **설치 파일**: AppId 와 설치 경로가 1.x 와 같아서 그 위에 덮어 설치됩니다.
  1.x 의 `_internal/` 폴더는 설치할 때 지웁니다(`[InstallDelete]`).
  레지스트리 키 구조가 같으므로 설치 중 `--register` 가 1.x 의 등록을 그대로 덮어씁니다.
- **CI**: `tests.yml` 은 fmt, clippy, 테스트 후 배포 폴더를 만들어 돌려 봅니다.
  `release.yml` 은 `v*` 태그에서 설치 파일과 포터블을 만들어 GitHub 릴리스에 올립니다.

## GUI (`crates/myzip/src/gui`)

egui / eframe 0.37 로 만들었습니다. 한 프로세스에 창은 하나입니다.

| 파일 | Python 판 | 하는 일 |
| --- | --- | --- |
| `browser.rs` | `ui/main_window.py` | 메인 창: 폴더 트리 + 목록(정렬, 여러 개 선택, 우클릭 메뉴), 인코딩 선택, 드래그&드롭, 미리보기, 텍스트 복사 |
| `batch.rs` | `app.py` 의 빠른 동작 | 탐색기 메뉴에서 바로 뜨는 작업 창: 양식 → 진행률 → 결과 |
| `forms.rs` | `ui/compress_dialog.py`, `ui/extract_dialog.py` | 압축하기 / 압축 풀기 양식 (설정 키 동일) |
| `settings.rs` | `ui/settings_dialog.py` | 확장자 연결, 탐색기 통합, 정보 |
| `task.rs`, `widgets.rs` | `ui/workers.py`, `ui/progress_dialog.py`, `ui/common.py` | 백그라운드 작업, 진행률, 암호 입력, 알림 |
| `style.rs` | — | 맑은 고딕(`C:\Windows\Fonts\malgun.ttf`) 을 읽어 한글을 그린다. exe 에 글꼴을 넣지 않는다 |

Python 판보다 나아진 점:

- **'압축 풀기...' 로 여러 개를 골라도 대화상자는 한 번만** 뜹니다. 고른 위치·옵션으로 모두 차례로 풉니다.
- **여러 아카이브의 암호**: 하나에 넣은 암호를 다음 아카이브에도 먼저 시도해 봅니다. 맞으면 다시 묻지 않습니다.
- **ZIP 암호를 열 때 바로 확인**합니다. Python 판은 틀린 암호를 받아 두었다가 풀 때 실패했습니다.
- **풀다가 실패하거나 취소하면** 이번에 새로 만든 빈 폴더를 지웁니다.
- 남은 시간은 작업마다 새로 잽니다 (암호를 기다린 시간은 빼고).

아이콘은 저장소의 `resources/icons/*.png` 를 exe 에 넣어 씁니다. 폴더 아이콘과 알림 표시는
egui 기본 글꼴에 📁 ⚠ 같은 글자가 없어서 직접 그립니다.

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
