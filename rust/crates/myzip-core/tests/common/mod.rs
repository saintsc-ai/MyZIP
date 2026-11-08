//! 통합 테스트 공용 도구.
#![allow(dead_code)]

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use myzip_core::{ExtractOptions, Progress, open_archive, walk_inputs, writer_for};

/// 한글 이름과 폴더 구조를 포함한 원본 트리를 만든다.
pub fn make_source(work: &Path) -> PathBuf {
    let src = work.join("원본자료");
    fs::create_dir_all(src.join("문서").join("하위폴더")).unwrap();
    fs::write(src.join("readme.txt"), "hello world\n").unwrap();
    fs::write(
        src.join("문서").join("보고서 2026.txt"),
        "한글 내용입니다.\n".repeat(100),
    )
    .unwrap();
    let pattern: Vec<u8> = (0..=255u8).collect();
    fs::write(
        src.join("문서").join("하위폴더").join("深い.dat"),
        pattern.repeat(500),
    )
    .unwrap();
    fs::write(src.join("big.bin"), vec![b'A'; 3 * 1024 * 1024]).unwrap();
    src
}

/// 상대경로 -> 크기 (폴더는 -1).
pub fn tree_snapshot(root: &Path) -> BTreeMap<String, i64> {
    walkdir::WalkDir::new(root)
        .min_depth(1)
        .into_iter()
        .map(|e| e.unwrap())
        .map(|e| {
            let rel = e.path().strip_prefix(root).unwrap();
            let key = rel
                .components()
                .map(|c| c.as_os_str().to_string_lossy().into_owned())
                .collect::<Vec<_>>()
                .join("/");
            let size = if e.file_type().is_dir() {
                -1
            } else {
                e.metadata().unwrap().len() as i64
            };
            (key, size)
        })
        .collect()
}

/// 원본 트리를 압축한다.
pub fn compress(src: &Path, archive: &Path, password: Option<&str>) {
    let items = walk_inputs(&[src]);
    let mut writer = writer_for(archive, 6, password).unwrap();
    writer.write(&items, &mut Progress::new()).unwrap();
}

/// 아카이브를 통째로 푼다.
pub fn extract_all(archive: &Path, dest: &Path, password: Option<&str>) -> Vec<String> {
    let mut reader = open_archive(archive, password).unwrap();
    let names = reader.entries().iter().map(|e| e.path.clone()).collect();
    reader
        .extract(dest, ExtractOptions::default(), &mut Progress::new())
        .unwrap();
    names
}

/// 폴더 안에 작업용 임시 파일(.myzip-part 등)이 남지 않았는지.
pub fn leftovers(root: &Path) -> Vec<String> {
    walkdir::WalkDir::new(root)
        .into_iter()
        .filter_map(|e| e.ok())
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .filter(|name| name.contains(".myzip-"))
        .collect()
}
