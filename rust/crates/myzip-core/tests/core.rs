//! 코어 압축/해제 동작 검증. Python 판 tests/test_core.py 를 옮겼다.

mod common;

use std::fs;
use std::io::Write;
use std::path::Path;

use common::{compress, extract_all, leftovers, make_source, tree_snapshot};
use myzip_core::{
    ConflictAction, Error, ExtractOptions, Progress, format_of, open_archive, walk_inputs,
    writer_for,
};

fn roundtrip(archive_name: &str, password: Option<&str>) {
    let work = tempfile::tempdir().unwrap();
    let src = make_source(work.path());
    let archive = work.path().join(archive_name);

    compress(&src, &archive, password);
    assert!(archive.metadata().unwrap().len() > 0, "{archive_name} 생성");

    let dest = work.path().join("out");
    let names = extract_all(&archive, &dest, password);

    assert_eq!(
        tree_snapshot(&src),
        tree_snapshot(&dest.join("원본자료")),
        "{archive_name} 왕복 일치"
    );
    assert!(
        names.iter().any(|n| n.contains("보고서 2026.txt")),
        "{archive_name} 한글 파일명 보존: {names:?}"
    );

    // 내용까지 바이트 단위로 같아야 한다.
    let original = fs::read(src.join("문서").join("하위폴더").join("深い.dat")).unwrap();
    let restored = fs::read(
        dest.join("원본자료")
            .join("문서")
            .join("하위폴더")
            .join("深い.dat"),
    )
    .unwrap();
    assert_eq!(original, restored, "{archive_name} 내용 일치");
    assert!(
        leftovers(work.path()).is_empty(),
        "임시 파일이 남음: {:?}",
        leftovers(work.path())
    );
}

#[test]
fn roundtrip_zip() {
    roundtrip("원본자료.zip", None);
}

#[test]
fn roundtrip_zip_aes() {
    roundtrip("원본자료-암호.zip", Some("비밀1234"));
}

#[test]
fn roundtrip_tar() {
    roundtrip("원본자료.tar", None);
}

#[test]
fn roundtrip_tgz() {
    roundtrip("원본자료.tgz", None);
}

#[test]
fn roundtrip_tar_bz2() {
    roundtrip("원본자료.tar.bz2", None);
}

#[test]
fn roundtrip_tar_xz() {
    roundtrip("원본자료.tar.xz", None);
}

/// CP949 로 기록된 구형 ZIP 의 파일명을 제대로 읽는지.
#[test]
fn cp949_zip() {
    let work = tempfile::tempdir().unwrap();
    let archive = work.path().join("legacy_cp949.zip");
    let korean = ["한글파일.txt", "문서/보고서.hwp", "사진/여름 휴가.jpg"];

    // zip 라이브러리는 non-ASCII 이름에 UTF-8 플래그를 켠다. 진짜 구형 아카이브를
    // 흉내 내려면 같은 길이의 ASCII 이름으로 쓴 다음 바이트를 바꿔치기해야 한다.
    let mut mapping = Vec::new();
    {
        let mut zw = zip::ZipWriter::new(fs::File::create(&archive).unwrap());
        for (i, name) in korean.iter().enumerate() {
            let raw = encoding_rs::EUC_KR.encode(name).0.into_owned();
            let placeholder: String = format!("N{i:03}")
                .chars()
                .chain(std::iter::repeat('x'))
                .take(raw.len())
                .collect();
            zw.start_file(
                placeholder.as_str(),
                zip::write::SimpleFileOptions::default(),
            )
            .unwrap();
            zw.write_all(b"data").unwrap();
            mapping.push((placeholder.into_bytes(), raw));
        }
        zw.finish().unwrap();
    }
    let mut blob = fs::read(&archive).unwrap();
    for (placeholder, raw) in &mapping {
        let mut i = 0;
        while let Some(pos) = blob[i..]
            .windows(placeholder.len())
            .position(|w| w == placeholder.as_slice())
        {
            let at = i + pos;
            blob[at..at + raw.len()].copy_from_slice(raw);
            i = at + raw.len();
        }
    }
    fs::write(&archive, blob).unwrap();

    let reader = open_archive(&archive, None).unwrap();
    let paths: Vec<&str> = reader.entries().iter().map(|e| e.path.as_str()).collect();
    assert_eq!(paths, korean, "CP949 ZIP 파일명 복원");
    assert_eq!(reader.encoding(), "cp949", "CP949 자동 판별");
}

/// 경로 탈출을 시도하는 ZIP 이 상위 폴더를 오염시키지 않는지.
#[test]
fn zip_slip_is_blocked() {
    let work = tempfile::tempdir().unwrap();
    let archive = work.path().join("evil.zip");
    {
        let mut zw = zip::ZipWriter::new(fs::File::create(&archive).unwrap());
        for (name, data) in [
            ("../../pwned.txt", b"x".as_slice()),
            ("C:/abs.txt", b"y"),
            ("normal.txt", b"ok"),
        ] {
            zw.start_file(name, zip::write::SimpleFileOptions::default())
                .unwrap();
            zw.write_all(data).unwrap();
        }
        zw.finish().unwrap();
    }

    let dest = work.path().join("slip").join("inner");
    let mut reader = open_archive(&archive, None).unwrap();
    reader
        .extract(&dest, ExtractOptions::default(), &mut Progress::new())
        .unwrap();

    assert!(!work.path().join("pwned.txt").exists());
    assert!(!work.path().join("slip").join("pwned.txt").exists());
    assert!(dest.join("pwned.txt").exists());
    assert!(dest.join("abs.txt").exists());
}

/// 확장자가 거짓일 때도 매직 바이트로 알아보는지.
#[test]
fn format_sniffing() {
    let work = tempfile::tempdir().unwrap();
    let src = make_source(work.path());
    let real = work.path().join("진짜.zip");
    compress(&src, &real, None);
    let fake = work.path().join("actually_a_zip.tar");
    fs::copy(&real, &fake).unwrap();
    assert_eq!(format_of(&fake), Some("zip"));

    let tgz = work.path().join("자료.tgz");
    compress(&src, &tgz, None);
    let renamed = work.path().join("이름만zip.zip");
    fs::copy(&tgz, &renamed).unwrap();
    assert_eq!(
        open_archive(&renamed, None).unwrap().format_name(),
        "TAR.GZ"
    );
}

/// 최상위 폴더 하나뿐이면 벗겨 내는 옵션.
#[test]
fn strip_root() {
    let work = tempfile::tempdir().unwrap();
    let src = make_source(work.path());
    let archive = work.path().join("원본자료.zip");
    compress(&src, &archive, None);

    let dest = work.path().join("stripped");
    let mut reader = open_archive(&archive, None).unwrap();
    let options = ExtractOptions {
        strip_root: true,
        ..ExtractOptions::default()
    };
    reader
        .extract(&dest, options, &mut Progress::new())
        .unwrap();
    assert!(dest.join("readme.txt").exists());
    assert!(dest.join("문서").join("보고서 2026.txt").exists());
}

/// AES 암호 ZIP 이 암호 없이는 안 열리는지.
#[test]
fn password() {
    let work = tempfile::tempdir().unwrap();
    let src = make_source(work.path()).join("readme.txt");
    let archive = work.path().join("secret.zip");
    let mut writer = writer_for(&archive, 6, Some("비밀1234")).unwrap();
    writer
        .write(&[(src.clone(), "readme.txt".into())], &mut Progress::new())
        .unwrap();

    let mut reader = open_archive(&archive, Some("비밀1234")).unwrap();
    let entry = reader.entries()[0].clone();
    assert!(entry.encrypted);
    assert_eq!(
        reader.read_bytes(&entry).unwrap(),
        fs::read(&src).unwrap(),
        "암호 ZIP 정상 해제"
    );

    let mut reader = open_archive(&archive, Some("틀린암호")).unwrap();
    let err = reader.read_bytes(&entry).unwrap_err();
    assert!(err.is_password(), "틀린 암호 거부: {err:?}");

    let mut reader = open_archive(&archive, None).unwrap();
    let err = reader.read_bytes(&entry).unwrap_err();
    assert!(err.is_password(), "암호 없이 거부: {err:?}");
}

/// 진행 중 취소가 실제로 멈추고, 반쪽짜리 파일을 남기지 않는지.
#[test]
fn cancel_extract() {
    let work = tempfile::tempdir().unwrap();
    let src = make_source(work.path());
    let archive = work.path().join("원본자료.zip");
    compress(&src, &archive, None);

    let dest = work.path().join("cancelled");
    let mut progress = Progress::new();
    let token = progress.token();
    progress.set_callback(move |p| {
        if p.done_bytes > 0 {
            token.cancel();
        }
    });

    let mut reader = open_archive(&archive, None).unwrap();
    let err = reader
        .extract(&dest, ExtractOptions::default(), &mut progress)
        .unwrap_err();
    assert!(
        matches!(err, Error::Cancelled(_)),
        "작업 취소 동작: {err:?}"
    );
    assert!(
        leftovers(&dest).is_empty(),
        "반쪽 파일이 남음: {:?}",
        leftovers(&dest)
    );
}

/// 압축 중 취소하면 아카이브 파일 자체가 생기지 않는다.
#[test]
fn cancel_compress() {
    let work = tempfile::tempdir().unwrap();
    let src = make_source(work.path());
    for name in ["취소.zip", "취소.tar.gz"] {
        let archive = work.path().join(name);
        let mut progress = Progress::new();
        let token = progress.token();
        progress.set_callback(move |p| {
            if p.done_bytes > 0 {
                token.cancel();
            }
        });
        let mut writer = writer_for(&archive, 6, None).unwrap();
        let err = writer
            .write(&walk_inputs(&[&src]), &mut progress)
            .unwrap_err();
        assert!(err.is_cancelled(), "{name}: {err:?}");
        assert!(!archive.exists(), "{name} 가 남음");
    }
    assert!(leftovers(work.path()).is_empty());
}

/// 이미 있는 파일과 겹칠 때 고른 대로 처리하는지.
#[test]
fn conflict_actions() {
    let work = tempfile::tempdir().unwrap();
    let src = make_source(work.path());
    let archive = work.path().join("원본자료.tar.gz");
    compress(&src, &archive, None);
    let dest = work.path().join("겹침");
    extract_all(&archive, &dest, None);

    let readme = dest.join("원본자료").join("readme.txt");
    fs::write(&readme, "수정본").unwrap();

    for (action, expect_readme, expect_copy) in [
        (ConflictAction::Skip, "수정본", false),
        (ConflictAction::Rename, "수정본", true),
        (ConflictAction::Overwrite, "hello world\n", true),
    ] {
        let mut asked = 0;
        let mut ask = |_: &Path, _: &myzip_core::Entry| {
            asked += 1;
            action
        };
        let mut reader = open_archive(&archive, None).unwrap();
        let options = ExtractOptions {
            on_conflict: Some(&mut ask),
            ..ExtractOptions::default()
        };
        reader
            .extract(&dest, options, &mut Progress::new())
            .unwrap();
        assert!(asked > 0);
        assert_eq!(
            fs::read_to_string(&readme).unwrap(),
            expect_readme,
            "{action:?}"
        );
        assert_eq!(
            dest.join("원본자료").join("readme (2).txt").exists(),
            expect_copy,
            "{action:?}"
        );
    }

    let mut cancel = |_: &Path, _: &myzip_core::Entry| ConflictAction::Cancel;
    let mut reader = open_archive(&archive, None).unwrap();
    let options = ExtractOptions {
        on_conflict: Some(&mut cancel),
        ..ExtractOptions::default()
    };
    assert!(
        reader
            .extract(&dest, options, &mut Progress::new())
            .unwrap_err()
            .is_cancelled()
    );
}

/// 일부 항목만 골라 풀기 (UI 에서 선택해서 풀 때).
#[test]
fn extract_selected_members() {
    let work = tempfile::tempdir().unwrap();
    let src = make_source(work.path());
    for name in ["골라.zip", "골라.tar.xz"] {
        let archive = work.path().join(name);
        compress(&src, &archive, None);
        let mut reader = open_archive(&archive, None).unwrap();
        let picked: Vec<_> = reader
            .entries()
            .iter()
            .filter(|e| e.path.ends_with(".txt"))
            .cloned()
            .collect();
        assert_eq!(picked.len(), 2);

        let dest = work.path().join(format!("골라-{}", name.replace('.', "_")));
        let options = ExtractOptions {
            members: Some(&picked),
            ..ExtractOptions::default()
        };
        reader
            .extract(&dest, options, &mut Progress::new())
            .unwrap();
        assert!(dest.join("원본자료").join("readme.txt").exists(), "{name}");
        assert!(!dest.join("원본자료").join("big.bin").exists(), "{name}");
    }
}

/// tar 로 감싸지 않은 단일 .gz/.bz2/.xz 도 열려야 한다.
#[test]
fn single_compressed_file() {
    let work = tempfile::tempdir().unwrap();
    let payload = "단일 파일 압축 내용\n".repeat(300).into_bytes();

    let write = |name: &str| {
        let path = work.path().join(name);
        let file = fs::File::create(&path).unwrap();
        if name.ends_with(".gz") {
            let mut enc = flate2::write::GzEncoder::new(file, flate2::Compression::default());
            enc.write_all(&payload).unwrap();
            enc.finish().unwrap();
        } else if name.ends_with(".bz2") {
            let mut enc = bzip2::write::BzEncoder::new(file, bzip2::Compression::default());
            enc.write_all(&payload).unwrap();
            enc.finish().unwrap();
        } else {
            let mut enc = liblzma::write::XzEncoder::new(file, 6);
            enc.write_all(&payload).unwrap();
            enc.finish().unwrap();
        }
        path
    };

    for name in ["보고서.txt.gz", "로그.txt.bz2", "덤프.sql.xz"] {
        let archive = write(name);
        let mut reader = open_archive(&archive, None).unwrap();
        assert_eq!(
            reader.format_name(),
            "단일 압축 파일",
            "{name} 단일 파일로 인식"
        );
        let entry = reader.entries()[0].clone();
        assert_eq!(
            entry.path,
            name[..name.rfind('.').unwrap()],
            "{name} 내부 이름 복원"
        );
        assert_eq!(entry.size, payload.len() as u64, "{name} 원본 크기");

        let dest = work.path().join(format!("single-{name}"));
        reader
            .extract(&dest, ExtractOptions::default(), &mut Progress::new())
            .unwrap();
        assert_eq!(
            fs::read(dest.join(&entry.path)).unwrap(),
            payload,
            "{name} 내용 일치"
        );
        assert!(
            reader.test(&mut Progress::new()).unwrap().is_empty(),
            "{name} 무결성"
        );
    }
}

/// 손상된 아카이브를 무결성 검사가 잡아내는지.
#[test]
fn integrity_test_detects_damage() {
    let work = tempfile::tempdir().unwrap();
    let src = make_source(work.path());
    for name in ["손상.zip", "손상.tar.gz"] {
        let archive = work.path().join(name);
        compress(&src, &archive, None);
        {
            let mut reader = open_archive(&archive, None).unwrap();
            assert!(
                reader.test(&mut Progress::new()).unwrap().is_empty(),
                "{name} 정상본"
            );
        }

        // 압축 데이터 한가운데를 망가뜨린다.
        let mut blob = fs::read(&archive).unwrap();
        let mid = blob.len() / 2;
        for b in &mut blob[mid..mid + 64] {
            *b ^= 0xFF;
        }
        fs::write(&archive, blob).unwrap();

        let bad = open_archive(&archive, None).and_then(|mut r| r.test(&mut Progress::new()));
        // 목록부터 못 읽어도(Err) 손상을 알린 것이다.
        if let Ok(list) = bad {
            assert!(!list.is_empty(), "{name} 손상 미검출");
        }
    }
}

/// 인코딩을 바꾸면 목록이 다시 만들어진다.
#[test]
fn reencode_names() {
    let work = tempfile::tempdir().unwrap();
    let archive = work.path().join("인코딩.tar");
    // 이름을 CP949 바이트 그대로 넣은 tar.
    {
        let mut builder = tar::Builder::new(fs::File::create(&archive).unwrap());
        let mut header = tar::Header::new_ustar();
        let raw = encoding_rs::EUC_KR.encode("한글.txt").0.into_owned();
        header.as_ustar_mut().unwrap().name[..raw.len()].copy_from_slice(&raw);
        header.set_size(2);
        header.set_mode(0o644);
        header.set_entry_type(tar::EntryType::Regular);
        header.set_cksum();
        builder.append(&header, b"hi".as_slice()).unwrap();
        builder.finish().unwrap();
    }
    let mut reader = open_archive(&archive, None).unwrap();
    assert_eq!(reader.encoding(), "cp949");
    assert_eq!(reader.entries()[0].path, "한글.txt");

    reader.set_encoding("cp1252").unwrap();
    assert_ne!(reader.entries()[0].path, "한글.txt");
    reader.set_encoding("cp949").unwrap();
    assert_eq!(reader.entries()[0].path, "한글.txt");
}

#[test]
fn unsupported_inputs() {
    let work = tempfile::tempdir().unwrap();
    let text = work.path().join("그냥.txt");
    fs::write(&text, "not an archive").unwrap();
    assert!(matches!(
        open_archive(&text, None),
        Err(Error::Unsupported(_))
    ));
    assert!(matches!(
        open_archive(&work.path().join("없음.zip"), None),
        Err(Error::Unsupported(_))
    ));
    assert!(writer_for(&work.path().join("x.rar"), 6, None).is_err());
}
