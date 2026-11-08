//! 7-Zip 엔진을 거치는 RAR/7Z 해제 검증. Python 판 tests/test_rar.py 를 옮겼다.
//!
//! 7z.exe 가 없으면 건너뛴다. 저장소의 bin/ 에 있으면 그것을 쓴다
//! (`python tools/fetch_7zip.py` 로 받을 수 있다).

mod common;

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use common::{extract_all, leftovers, make_source, tree_snapshot};
use myzip_core::{Error, ExtractOptions, Progress, open_archive, sevenzip, writer_for};

fn seven_zip() -> Option<PathBuf> {
    let found = sevenzip::find_7z(false);
    if found.is_none() {
        eprintln!("건너뜀: 7z.exe 를 찾지 못했습니다");
    }
    found
}

fn build_with_7z(exe: &Path, src: &Path, archive: &Path, extra: &[&str]) {
    let status = Command::new(exe)
        .arg("a")
        .arg(archive)
        .arg(src)
        .args(extra)
        .args(["-y", "-bso0", "-bsp0"])
        .status()
        .unwrap();
    assert!(
        status.success() && archive.exists(),
        "7z 로 테스트 파일을 만들지 못했습니다"
    );
}

#[test]
fn sevenzip_roundtrip() {
    let Some(exe) = seven_zip() else { return };
    let work = tempfile::tempdir().unwrap();
    let src = make_source(work.path());
    let archive = work.path().join("샘플.7z");
    build_with_7z(&exe, &src, &archive, &[]);

    let reader = open_archive(&archive, None).unwrap();
    assert_eq!(reader.format_name(), "7Z");
    assert!(!reader.can_change_encoding());
    drop(reader);

    let dest = work.path().join("out");
    let names = extract_all(&archive, &dest, None);
    assert!(
        names.iter().any(|n| n.ends_with("보고서 2026.txt")),
        "{names:?}"
    );
    assert_eq!(tree_snapshot(&src), tree_snapshot(&dest.join("원본자료")));
    assert!(
        leftovers(work.path()).is_empty(),
        "{:?}",
        leftovers(work.path())
    );
}

#[test]
fn sevenzip_progress_and_strip_root() {
    let Some(exe) = seven_zip() else { return };
    let work = tempfile::tempdir().unwrap();
    let src = make_source(work.path());
    let archive = work.path().join("샘플.7z");
    build_with_7z(&exe, &src, &archive, &[]);

    let mut reader = open_archive(&archive, None).unwrap();
    let mut progress = Progress::new();
    let dest = work.path().join("stripped");
    let options = ExtractOptions {
        strip_root: true,
        ..ExtractOptions::default()
    };
    reader.extract(&dest, options, &mut progress).unwrap();
    assert!(dest.join("readme.txt").exists());
    assert_eq!(progress.done_files, progress.total_files);
}

#[test]
fn sevenzip_password() {
    let Some(exe) = seven_zip() else { return };
    let work = tempfile::tempdir().unwrap();
    let src = make_source(work.path());
    let archive = work.path().join("샘플-암호.7z");
    build_with_7z(&exe, &src, &archive, &["-p비밀1234"]);

    let mut reader = open_archive(&archive, Some("비밀1234")).unwrap();
    let target = reader
        .entries()
        .iter()
        .find(|e| e.path.ends_with("readme.txt"))
        .unwrap()
        .clone();
    assert!(target.encrypted);
    assert_eq!(
        reader.read_bytes(&target).unwrap(),
        fs::read(src.join("readme.txt")).unwrap()
    );

    let mut reader = open_archive(&archive, Some("틀린암호")).unwrap();
    let err = reader.read_bytes(&target).unwrap_err();
    assert!(err.is_password(), "틀린 암호 거부: {err:?}");
}

/// 헤더까지 암호화된 아카이브는 목록조차 못 읽는다. 이때 일반 오류가 아니라
/// PasswordRequired 로 알려야 UI 가 오류창 대신 암호 입력을 띄울 수 있다.
#[test]
fn header_encrypted() {
    let Some(exe) = seven_zip() else { return };
    let work = tempfile::tempdir().unwrap();
    let src = make_source(work.path());
    let archive = work.path().join("헤더암호.7z");
    build_with_7z(&exe, &src, &archive, &["-mhe=on", "-p테스트암호"]);

    match open_archive(&archive, None) {
        Err(Error::PasswordRequired(_)) => {}
        Err(other) => panic!("엉뚱한 오류: {other:?}"),
        Ok(_) => panic!("암호 없이 열렸다"),
    }
    let reader = open_archive(&archive, Some("테스트암호")).unwrap();
    assert!(!reader.entries().is_empty());
}

#[test]
fn integrity_test() {
    let Some(exe) = seven_zip() else { return };
    let work = tempfile::tempdir().unwrap();
    let src = make_source(work.path());
    let archive = work.path().join("검사.7z");
    build_with_7z(&exe, &src, &archive, &[]);

    let mut reader = open_archive(&archive, None).unwrap();
    assert!(reader.test(&mut Progress::new()).unwrap().is_empty());
}

// --- RAR 4.x 픽스처 ---------------------------------------------------------
//
// RAR '압축'은 만들 수 없지만(라이선스·독점 알고리즘), 무압축 저장(method 0x30)
// 방식의 RAR 컨테이너는 문서화된 헤더 구조만으로 조립할 수 있다.
// tests/rar_fixture.py 와 같은 구조다.

fn crc32(data: &[u8]) -> u32 {
    let mut crc = flate2::Crc::new();
    crc.update(data);
    crc.sum()
}

fn with_crc(body: Vec<u8>) -> Vec<u8> {
    let mut out = ((crc32(&body) & 0xFFFF) as u16).to_le_bytes().to_vec();
    out.extend(body);
    out
}

fn rar_file_header(name_bytes: &[u8], data: &[u8], is_dir: bool) -> Vec<u8> {
    let mut flags: u16 = 0x8000; // LONG_BLOCK
    let mut attr: u32 = 0x20;
    if is_dir {
        flags |= 0x00E0;
        attr = 0x10;
    }
    // 2026-08-31 12:30:00 (DOS 시각)
    let dos_time: u32 = ((2026 - 1980) << 25) | (8 << 21) | (31 << 16) | (12 << 11) | (30 << 5);
    let mut body = vec![0x74];
    body.extend(flags.to_le_bytes());
    body.extend(((32 + name_bytes.len()) as u16).to_le_bytes());
    body.extend((data.len() as u32).to_le_bytes()); // PACK_SIZE
    body.extend((data.len() as u32).to_le_bytes()); // UNP_SIZE
    body.push(2); // HOST_OS = Win32
    body.extend(crc32(data).to_le_bytes());
    body.extend(dos_time.to_le_bytes());
    body.push(20); // UNP_VER
    body.push(0x30); // METHOD = store
    body.extend((name_bytes.len() as u16).to_le_bytes());
    body.extend(attr.to_le_bytes());
    body.extend(name_bytes);
    let mut out = with_crc(body);
    out.extend(data);
    out
}

fn build_rar(path: &Path, dirs: &[&[u8]], files: &[(&[u8], &[u8])]) {
    let mut blob = vec![0x52, 0x61, 0x72, 0x21, 0x1A, 0x07, 0x00];
    let mut main = vec![0x73];
    main.extend(0u16.to_le_bytes());
    main.extend(13u16.to_le_bytes());
    main.extend(0u16.to_le_bytes());
    main.extend(0u32.to_le_bytes());
    blob.extend(with_crc(main));
    for dir in dirs {
        blob.extend(rar_file_header(dir, b"", true));
    }
    for (name, data) in files {
        blob.extend(rar_file_header(name, data, false));
    }
    let mut end = vec![0x7B];
    end.extend(0x4000u16.to_le_bytes());
    end.extend(7u16.to_le_bytes());
    blob.extend(with_crc(end));
    fs::write(path, blob).unwrap();
}

#[test]
fn rar_files() {
    if seven_zip().is_none() {
        return;
    }
    let work = tempfile::tempdir().unwrap();
    let text = "hello rar\n".repeat(40).into_bytes();
    let binary: Vec<u8> = (0..=255u8).collect::<Vec<_>>().repeat(300);
    let archive = work.path().join("조립.rar");
    build_rar(
        &archive,
        &[b"sample", b"sample\\inner"],
        &[
            (b"sample\\readme.txt", &text),
            (b"sample\\inner\\binary.dat", &binary),
        ],
    );

    let mut reader = open_archive(&archive, None).unwrap();
    assert_eq!(reader.format_name(), "RAR");
    let mut names: Vec<&str> = reader.entries().iter().map(|e| e.path.as_str()).collect();
    names.sort();
    assert_eq!(
        names,
        [
            "sample",
            "sample/inner",
            "sample/inner/binary.dat",
            "sample/readme.txt"
        ]
    );

    let dest = work.path().join("rar-out");
    reader
        .extract(&dest, ExtractOptions::default(), &mut Progress::new())
        .unwrap();
    assert_eq!(
        fs::read(dest.join("sample").join("readme.txt")).unwrap(),
        text
    );
    assert_eq!(
        fs::read(dest.join("sample").join("inner").join("binary.dat")).unwrap(),
        binary
    );
    assert!(leftovers(&dest).is_empty());
}

#[cfg(windows)]
fn oem_codepage() -> u32 {
    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn GetOEMCP() -> u32;
    }
    // SAFETY: 인자 없는 조회 함수다.
    unsafe { GetOEMCP() }
}

#[cfg(not(windows))]
fn oem_codepage() -> u32 {
    0
}

/// RAR4 는 이름을 OEM 코드페이지로 저장한다. 픽스처는 CP949 로 쓰므로
/// OEM 코드페이지가 949 인 PC(한국어 Windows)에서만 의미가 있다.
#[test]
fn rar_korean_names() {
    if seven_zip().is_none() {
        return;
    }
    if oem_codepage() != 949 {
        eprintln!("건너뜀: OEM 코드페이지가 {} 입니다", oem_codepage());
        return;
    }
    let work = tempfile::tempdir().unwrap();
    let text = "한글 내용 테스트\n".repeat(40).into_bytes();
    let dir = encoding_rs::EUC_KR.encode("샘플").0.into_owned();
    let file = encoding_rs::EUC_KR
        .encode("샘플\\읽어보기.txt")
        .0
        .into_owned();
    let archive = work.path().join("조립-한글.rar");
    build_rar(&archive, &[&dir], &[(&file, &text)]);

    let dest = work.path().join("out");
    let names = extract_all(&archive, &dest, None);
    assert!(names.iter().any(|n| n == "샘플/읽어보기.txt"), "{names:?}");
    assert_eq!(
        fs::read(dest.join("샘플").join("읽어보기.txt")).unwrap(),
        text
    );
}

#[test]
fn rar_write_refused() {
    let err = writer_for(Path::new("만들기.rar"), 6, None)
        .err()
        .expect("RAR 압축은 거부해야 한다");
    assert!(matches!(err, Error::Unsupported(_)));
}
