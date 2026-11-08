//! 아카이브 포맷 공통 인터페이스와 공용 해제 절차.

use std::fs::{self, File};
use std::path::{Path, PathBuf};

use crate::entry::{ConflictAction, Entry, Progress, apply_mtime};
use crate::error::{Error, Result};
use crate::path_safety::{safe_join, unique_path};

/// 덮어쓰기 충돌이 났을 때 물어볼 곳.
pub type ConflictFn<'a> = &'a mut dyn FnMut(&Path, &Entry) -> ConflictAction;

/// [`ArchiveReader::extract`] 의 선택 사항.
#[derive(Default)]
pub struct ExtractOptions<'a> {
    /// None 이면 전체.
    pub members: Option<&'a [Entry]>,
    /// 아카이브 최상위가 폴더 하나뿐이면 그 폴더를 벗겨 낸다.
    pub strip_root: bool,
    /// 이미 파일이 있을 때 호출. 없으면 덮어쓴다.
    pub on_conflict: Option<ConflictFn<'a>>,
}

/// 읽기 전용 아카이브 접근자.
///
/// 항목 목록은 열 때 한 번 만들어 둔다. 인코딩을 바꾸면 다시 만든다.
/// UI 는 작업을 별도 스레드에서 돌리므로 `Send` 여야 한다.
pub trait ArchiveReader: Send {
    fn path(&self) -> &Path;

    /// 사람이 읽는 포맷 이름 ("ZIP", "TAR.GZ" ...).
    fn format_name(&self) -> &str;

    fn entries(&self) -> &[Entry];

    /// 현재 파일명 해석에 쓰는 인코딩.
    fn encoding(&self) -> &str;

    /// 파일명 인코딩을 바꾸고 목록을 다시 만든다.
    fn set_encoding(&mut self, encoding: &str) -> Result<()>;

    /// 사용자가 인코딩을 고를 수 있는 포맷인지. 7-Zip 엔진은 스스로 처리한다.
    fn can_change_encoding(&self) -> bool {
        true
    }

    /// 항목 하나를 dest 파일 경로로 꺼낸다. 상위 폴더는 이미 만들어져 있다.
    fn extract_one(&mut self, entry: &Entry, dest: &Path, progress: &mut Progress) -> Result<()>;

    /// 항목 내용을 메모리로 읽는다. 미리보기용.
    fn read_bytes(&mut self, entry: &Entry) -> Result<Vec<u8>>;

    /// 무결성 검사. 손상된 항목 경로(또는 메시지) 목록을 돌려준다.
    fn test(&mut self, progress: &mut Progress) -> Result<Vec<String>>;

    fn has_encrypted(&self) -> bool {
        self.entries().iter().any(|e| e.encrypted)
    }

    fn total_size(&self) -> u64 {
        self.entries()
            .iter()
            .filter(|e| !e.is_dir)
            .map(|e| e.size)
            .sum()
    }

    /// 항목들을 dest_dir 아래로 꺼낸다. 실제로 만들어진 경로 목록을 돌려준다.
    fn extract(
        &mut self,
        dest_dir: &Path,
        options: ExtractOptions<'_>,
        progress: &mut Progress,
    ) -> Result<Vec<PathBuf>> {
        let all = self.entries().to_vec();
        let targets = options
            .members
            .map_or_else(|| all.clone(), <[Entry]>::to_vec);
        let mut job = ExtractJob::new(
            dest_dir,
            &all,
            &targets,
            options.strip_root,
            options.on_conflict,
            progress,
        )?;
        for entry in &targets {
            job.process(entry, |dest, progress| {
                self.extract_one(entry, dest, progress)
            })?;
        }
        Ok(job.finish())
    }
}

/// 아카이브 생성기.
pub trait ArchiveWriter {
    fn format_name(&self) -> &str;

    fn supports_password(&self) -> bool {
        false
    }

    /// (원본 절대경로, 아카이브 내부 경로) 쌍들을 기록한다.
    ///
    /// 실패하거나 취소되면 반쪽짜리 아카이브를 남기지 않는다.
    fn write(&mut self, items: &[(PathBuf, String)], progress: &mut Progress) -> Result<()>;
}

/// 해제 한 번의 공통 절차: 경로 검증, 충돌 처리, 임시 파일, 시각 복원.
///
/// 포맷마다 항목을 꺼내는 방법만 다르다. TAR 는 앞에서부터 흘려 읽어야 하고,
/// 7-Zip 은 한꺼번에 풀어 놓은 것을 옮겨야 한다. 그 차이는 `process` 에
/// 넘기는 클로저가 맡고, 나머지는 여기서 똑같이 처리한다.
pub(crate) struct ExtractJob<'p, 'c> {
    dest_dir: PathBuf,
    prefix: String,
    on_conflict: Option<ConflictFn<'c>>,
    pub progress: &'p mut Progress,
    written: Vec<PathBuf>,
}

impl<'p, 'c> ExtractJob<'p, 'c> {
    pub fn new(
        dest_dir: &Path,
        all: &[Entry],
        targets: &[Entry],
        strip_root: bool,
        on_conflict: Option<ConflictFn<'c>>,
        progress: &'p mut Progress,
    ) -> Result<Self> {
        fs::create_dir_all(dest_dir)?;
        progress.total_files = targets.len() as u64;
        progress.total_bytes = targets.iter().filter(|e| !e.is_dir).map(|e| e.size).sum();
        Ok(Self {
            dest_dir: dest_dir.to_path_buf(),
            prefix: if strip_root {
                common_root(all)
            } else {
                String::new()
            },
            on_conflict,
            progress,
            written: Vec::new(),
        })
    }

    /// 최상위 폴더를 벗긴 상대 경로. 벗기고 나서 남는 게 없으면 None.
    fn relative<'e>(&self, path: &'e str) -> Option<&'e str> {
        if self.prefix.is_empty() {
            return Some(path);
        }
        match path.strip_prefix(self.prefix.as_str()) {
            Some(rest) => {
                let rest = rest.trim_start_matches('/');
                (!rest.is_empty()).then_some(rest)
            }
            None => Some(path),
        }
    }

    /// 항목 하나를 처리한다. `write` 는 받은 경로에 파일 내용을 만든다.
    pub fn process<F>(&mut self, entry: &Entry, write: F) -> Result<()>
    where
        F: FnOnce(&Path, &mut Progress) -> Result<()>,
    {
        self.progress.check_cancel()?;
        let Some(rel) = self.relative(&entry.path) else {
            // 벗겨 낸 최상위 폴더 자체. 할 일은 없지만 진행률에는 센다.
            return self.progress.advance(0, 1);
        };
        let mut out = safe_join(&self.dest_dir, rel)?;
        self.progress.set_current(&entry.path);

        if entry.is_dir {
            fs::create_dir_all(&out)?;
            self.progress.advance(0, 1)?;
            self.written.push(out);
            return Ok(());
        }

        if let Some(parent) = out.parent() {
            fs::create_dir_all(parent)?;
        }

        if out.symlink_metadata().is_ok() {
            let action = match self.on_conflict.as_mut() {
                Some(ask) => ask(&out, entry),
                None => ConflictAction::Overwrite,
            };
            match action {
                ConflictAction::Skip => {
                    self.progress.advance(entry.size, 1)?;
                    return Ok(());
                }
                ConflictAction::Rename => out = unique_path(&out)?,
                ConflictAction::Cancel => {
                    return Err(Error::Cancelled("덮어쓰기 확인에서 취소했습니다.".into()));
                }
                ConflictAction::Overwrite => {}
            }
        }

        // 중간에 실패하거나 취소되면 반쪽짜리 파일이 남지 않게 임시로 만든 뒤 옮긴다.
        let part = part_path(&out);
        let result = write(&part, self.progress).and_then(|()| {
            if part.symlink_metadata().is_ok() {
                fs::rename(&part, &out)?;
                Ok(true)
            } else {
                Ok(false) // 장치 파일 등 만들지 않은 항목
            }
        });
        let created = match result {
            Ok(created) => created,
            Err(err) => {
                let _ = fs::remove_file(&part);
                return Err(err);
            }
        };

        if created {
            apply_mtime(&out, entry.mtime);
            self.written.push(out);
        }
        self.progress.advance(0, 1)?;
        Ok(())
    }

    pub fn finish(self) -> Vec<PathBuf> {
        self.written
    }
}

/// 작업 중인 파일의 임시 이름.
pub(crate) fn part_path(target: &Path) -> PathBuf {
    let mut name = target.file_name().unwrap_or_default().to_os_string();
    name.push(".myzip-part");
    target.with_file_name(name)
}

/// 임시 파일에 다 쓴 뒤에만 target 으로 옮긴다.
pub(crate) fn write_atomically<F>(target: &Path, write: F) -> Result<()>
where
    F: FnOnce(File) -> Result<()>,
{
    let part = part_path(target);
    let file = File::create(&part)?;
    match write(file) {
        Ok(()) => {
            fs::rename(&part, target)?;
            Ok(())
        }
        Err(err) => {
            let _ = fs::remove_file(&part);
            Err(err)
        }
    }
}

/// 모든 항목이 같은 최상위 폴더 하나에 들어 있으면 그 이름을 돌려준다.
pub fn common_root(entries: &[Entry]) -> String {
    let mut root: Option<&str> = None;
    for entry in entries {
        let head = entry.path.split('/').next().unwrap_or("");
        if head.is_empty() {
            continue;
        }
        match root {
            None => root = Some(head),
            Some(r) if r != head => return String::new(),
            _ => {}
        }
    }
    let Some(root) = root else {
        return String::new();
    };
    // 최상위에 파일이 하나 덜렁 있는 경우는 벗기면 안 된다
    if entries.iter().any(|e| e.path == root && !e.is_dir) {
        return String::new();
    }
    root.to_owned()
}

/// 입력 경로들을 (절대경로, 아카이브 내부 경로) 쌍으로 펼친다.
///
/// 폴더는 재귀적으로 들어가고, 각 입력의 이름이 아카이브 최상위가 된다.
pub fn walk_inputs<P: AsRef<Path>>(paths: &[P]) -> Vec<(PathBuf, String)> {
    let mut items = Vec::new();
    for raw in paths {
        let path = match dunce::canonicalize(raw.as_ref()) {
            Ok(p) => p,
            Err(_) => continue, // 없는 경로
        };
        let name = path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| "_".into());

        if path.is_dir() {
            items.push((path.clone(), name.clone()));
            let walker = walkdir::WalkDir::new(&path)
                .min_depth(1)
                .sort_by_file_name();
            for sub in walker.into_iter().filter_map(|e| e.ok()) {
                let rel = sub.path().strip_prefix(&path).unwrap_or(sub.path());
                let mut arc = name.clone();
                for comp in rel.components() {
                    arc.push('/');
                    arc.push_str(&comp.as_os_str().to_string_lossy());
                }
                items.push((sub.into_path(), arc));
            }
        } else {
            items.push((path, name));
        }
    }
    items
}

#[cfg(test)]
mod tests {
    use super::*;

    fn e(path: &str, is_dir: bool) -> Entry {
        Entry {
            path: path.into(),
            is_dir,
            ..Entry::default()
        }
    }

    #[test]
    fn common_root_rules() {
        assert_eq!(common_root(&[e("a", true), e("a/b.txt", false)]), "a");
        assert_eq!(common_root(&[e("a/b.txt", false), e("c.txt", false)]), "");
        // 최상위 파일 하나뿐이면 벗기지 않는다
        assert_eq!(common_root(&[e("a", false)]), "");
    }
}
