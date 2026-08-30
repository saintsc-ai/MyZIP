# 제3자 구성요소 고지

MyZIP 의 소스 코드는 MIT 라이선스입니다([LICENSE](LICENSE)).
그 라이선스는 소스 코드에만 적용되며, 배포판(`setup.exe`, `portable.zip`)에
함께 들어가는 아래 구성요소는 각자의 라이선스를 따릅니다.

## PySide6 / Qt 6

- 라이선스: LGPL v3
- 출처: https://www.qt.io/qt-for-python
- MyZIP 은 Qt 를 동적 링크로만 사용하며 Qt 소스를 수정하지 않습니다.
  LGPL v3 에 따라 사용자는 Qt 라이브러리를 교체할 수 있습니다 —
  배포 폴더의 `_internal/PySide6/` 안 DLL 을 바꾸면 됩니다.
- Qt 소스: https://download.qt.io/official_releases/qt/

## 7-Zip (7z.exe, 7z.dll)

- 라이선스: LGPL v2.1 이상 + unRAR 제한 라이선스
- 출처: https://www.7-zip.org
- 저장소에는 포함하지 않고 `tools/fetch_7zip.py` 가 공식 MSI 에서
  내려받습니다. 배포판에는 `_internal/bin/` 에 함께 넣습니다.

**unRAR 라이선스 관련**: 7-Zip 의 RAR 해제 코드는 RARLAB 의 unRAR 소스에서
왔고, 그 라이선스는 해당 코드를 **RAR 압축기(아카이버) 제작에 사용하는 것을
금지**합니다. MyZIP 은 RAR 을 해제하는 데만 사용하며 RAR 압축 기능을
제공하지 않습니다. 7-Zip 과 반디집도 같은 제약을 받습니다.

원문: <https://www.7-zip.org/license.txt>

## pyzipper

- 라이선스: MIT
- 출처: https://github.com/danifus/pyzipper
- ZIP AES-256 암호화/복호화에 사용합니다.

## Inno Setup (빌드 도구)

- 라이선스: Inno Setup License (수정 BSD 계열)
- 출처: https://jrsoftware.org/isinfo.php
- 설치 파일을 만드는 데만 쓰이며, 배포판에 포함되지 않습니다.
