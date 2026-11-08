# 제3자 구성요소 고지

MyZIP 의 소스 코드는 MIT 라이선스입니다([LICENSE](LICENSE)).
그 라이선스는 소스 코드에만 적용되며, 배포판(`setup.exe`, `portable.zip`)에
함께 들어가는 아래 구성요소는 각자의 라이선스를 따릅니다.

## Rust 라이브러리 (MyZIP.exe 안에 정적으로 링크)

MyZIP.exe 는 Rust 로 만들었고 아래 라이브러리를 함께 컴파일해 넣었습니다.
대부분 `MIT OR Apache-2.0` 이며, 그 밖의 라이선스는 다음과 같습니다.
전체 목록은 소스의 `rust/Cargo.lock` 에 있고, 각 라이브러리의 라이선스 원문은
<https://crates.io> 의 해당 페이지에서 볼 수 있습니다.

| 구성요소 | 쓰임 | 라이선스 |
| --- | --- | --- |
| egui / eframe | 화면 | MIT OR Apache-2.0 |
| zip | ZIP 읽기·쓰기, AES-256 | MIT |
| tar, flate2, zlib-rs | TAR, GZIP | MIT OR Apache-2.0, Zlib |
| bzip2 (libbz2-rs-sys) | BZIP2 | bzip2-1.0.6 (BSD 계열) |
| liblzma (xz) | XZ | MIT OR Apache-2.0 / xz 는 0BSD |
| encoding_rs | 한글·일본어·중국어 파일명 인코딩 | (MIT OR Apache-2.0) AND BSD-3-Clause |
| epaint_default_fonts | egui 기본 글꼴(Ubuntu, Hack) | OFL-1.1, Ubuntu Font Licence 1.0 |
| winreg, windows-sys, rfd 등 | Windows API, 레지스트리, 파일 선택 창 | MIT 또는 Apache-2.0 |

한글은 Windows 에 들어 있는 맑은 고딕을 실행할 때 읽어 쓰며, 배포판에 넣지 않습니다.

## 7-Zip (bin/7z.exe, bin/7z.dll)

- 라이선스: LGPL v2.1 이상 + unRAR 제한 라이선스
- 출처: <https://www.7-zip.org>
- 저장소에는 포함하지 않고 `tools/fetch_7zip.py` 가 공식 MSI 에서
  내려받습니다. 배포판에는 `bin/` 폴더에 별도 파일로 넣으므로 사용자가 교체할 수 있습니다.

**unRAR 라이선스 관련**: 7-Zip 의 RAR 해제 코드는 RARLAB 의 unRAR 소스에서
왔고, 그 라이선스는 해당 코드를 **RAR 압축기(아카이버) 제작에 사용하는 것을
금지**합니다. MyZIP 은 RAR 을 해제하는 데만 사용하며 RAR 압축 기능을
제공하지 않습니다. 7-Zip 과 반디집도 같은 제약을 받습니다.

원문: <https://www.7-zip.org/license.txt>

## Inno Setup (빌드 도구)

- 라이선스: Inno Setup License (수정 BSD 계열)
- 출처: <https://jrsoftware.org/isinfo.php>
- 설치 파일을 만드는 데만 쓰이며, 배포판에 포함되지 않습니다.
