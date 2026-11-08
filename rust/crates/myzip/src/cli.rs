//! 명령줄 해석.
//!
//! 탐색기 컨텍스트 메뉴가 호출하는 얼굴이기도 하다. 옵션 이름은 Python 판과
//! 같다. 레지스트리에 적힌 명령을 그대로 이어받아야 하기 때문이다.

use std::ffi::OsString;
use std::path::PathBuf;

use myzip_core::{APP_NAME, VERSION};

/// 아카이브/파일에 대해 할 일.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Mode {
    /// 메인 창으로 열기
    Open,
    /// 아카이브가 있는 폴더에 바로 풀기
    ExtractHere,
    /// 아카이브 이름의 폴더를 만들어 풀기
    ExtractTo,
    /// 대화상자로 위치를 정해 풀기
    ExtractDialog,
    /// 무결성 검사
    Test,
    /// 압축 대화상자
    CompressDialog,
    CompressZip,
    CompressTgz,
}

impl Mode {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Open => "open",
            Self::ExtractHere => "extract-here",
            Self::ExtractTo => "extract-to",
            Self::ExtractDialog => "extract",
            Self::Test => "test",
            Self::CompressDialog => "compress",
            Self::CompressZip => "compress-zip",
            Self::CompressTgz => "compress-tgz",
        }
    }

    pub fn parse(name: &str) -> Option<Self> {
        [
            Self::Open,
            Self::ExtractHere,
            Self::ExtractTo,
            Self::ExtractDialog,
            Self::Test,
            Self::CompressDialog,
            Self::CompressZip,
            Self::CompressTgz,
        ]
        .into_iter()
        .find(|m| m.as_str() == name)
    }
}

/// 한 번의 작업 요청. 다른 인스턴스로 넘길 수도 있다.
#[derive(Debug, Clone, PartialEq)]
pub struct Request {
    pub mode: Mode,
    /// 절대 경로. 다른 프로세스로 넘어가도 의미가 같아야 한다.
    pub paths: Vec<PathBuf>,
    pub password: Option<String>,
}

#[derive(Debug, PartialEq)]
pub enum Command {
    Help,
    Version,
    /// 확장자 연결과 탐색기 메뉴 설치
    Register,
    /// 탐색기 메뉴만 설치 (확장자 연결은 건드리지 않음)
    RegisterMenu,
    Unregister,
    /// 아카이브 목록을 콘솔에 출력
    List(Request),
    Run(Request),
}

pub fn help_text() -> String {
    format!(
        "{APP_NAME} {VERSION} - 압축 프로그램

사용법: MyZIP [동작] [파일...] [--password 암호]

동작 (하나만 고를 수 있습니다):
  (없음)               아카이브를 창으로 열기
  --extract-here       아카이브가 있는 폴더에 바로 풀기
  --extract-to         아카이브 이름의 폴더를 만들어 풀기 (여러 개면 각각)
  --extract            압축 풀기 대화상자 열기
  --test               무결성 검사
  --list               아카이브 안의 목록을 출력
  --compress           압축 대화상자 열기
  --compress-zip       바로 ZIP 으로 압축 (여러 개면 하나로 묶음)
  --compress-tgz       바로 TAR.GZ 로 압축 (여러 개면 하나로 묶음)
  --register           확장자 연결과 탐색기 메뉴 설치
  --register-menu      탐색기 메뉴만 설치 (확장자 연결은 건드리지 않음)
  --unregister         확장자 연결과 탐색기 메뉴 제거

옵션:
  -p, --password 암호  아카이브 암호 (ZIP 압축 시 AES-256 으로 암호화)
  -V, --version        버전 출력
  -h, --help           이 도움말 출력
"
    )
}

#[derive(Clone, Copy)]
enum Action {
    Mode(Mode),
    List,
    Register,
    RegisterMenu,
    Unregister,
}

fn action_for(flag: &str) -> Option<Action> {
    Some(match flag {
        "--extract-here" => Action::Mode(Mode::ExtractHere),
        "--extract-to" => Action::Mode(Mode::ExtractTo),
        "--extract" => Action::Mode(Mode::ExtractDialog),
        "--test" => Action::Mode(Mode::Test),
        "--compress" => Action::Mode(Mode::CompressDialog),
        "--compress-zip" => Action::Mode(Mode::CompressZip),
        "--compress-tgz" => Action::Mode(Mode::CompressTgz),
        "--list" => Action::List,
        "--register" => Action::Register,
        "--register-menu" => Action::RegisterMenu,
        "--unregister" => Action::Unregister,
        _ => return None,
    })
}

/// 명령줄을 해석한다. 오류면 사용자에게 보여 줄 메시지를 돌려준다.
pub fn parse<I>(args: I) -> Result<Command, String>
where
    I: IntoIterator<Item = OsString>,
{
    let mut action: Option<(Action, String)> = None;
    let mut password: Option<String> = None;
    let mut paths: Vec<PathBuf> = Vec::new();
    let mut only_paths = false;

    let mut iter = args.into_iter();
    while let Some(raw) = iter.next() {
        // 경로는 유니코드가 아닐 수도 있으니 OsString 그대로 다룬다.
        let text = raw.to_string_lossy();
        if only_paths || !text.starts_with('-') || text == "-" {
            paths.push(PathBuf::from(raw));
            continue;
        }
        match text.as_ref() {
            "--" => only_paths = true,
            "-h" | "--help" => return Ok(Command::Help),
            "-V" | "--version" => return Ok(Command::Version),
            "-p" | "--password" => {
                let value = iter
                    .next()
                    .ok_or_else(|| format!("{text} 다음에 암호를 적어 주세요."))?;
                password = Some(value.to_string_lossy().into_owned());
            }
            flag if flag.starts_with("--password=") => {
                password = Some(flag["--password=".len()..].to_owned());
            }
            flag => {
                let found =
                    action_for(flag).ok_or_else(|| format!("알 수 없는 옵션입니다: {flag}"))?;
                if let Some((_, previous)) = &action {
                    return Err(format!("{previous} 와 {flag} 는 함께 쓸 수 없습니다."));
                }
                action = Some((found, flag.to_owned()));
            }
        }
    }

    // 다른 프로세스나 작업 폴더가 달라도 같은 파일을 가리키도록 절대 경로로 바꾼다.
    let paths: Vec<PathBuf> = paths
        .into_iter()
        .map(|p| std::path::absolute(&p).unwrap_or(p))
        .collect();
    let password = password.filter(|p| !p.is_empty());
    let request = |mode| Request {
        mode,
        paths: paths.clone(),
        password: password.clone(),
    };

    Ok(match action.map(|(a, _)| a) {
        None => Command::Run(request(Mode::Open)),
        Some(Action::Mode(mode)) => Command::Run(request(mode)),
        Some(Action::List) => Command::List(request(Mode::Open)),
        Some(Action::Register) => Command::Register,
        Some(Action::RegisterMenu) => Command::RegisterMenu,
        Some(Action::Unregister) => Command::Unregister,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(list: &[&str]) -> Vec<OsString> {
        list.iter().map(OsString::from).collect()
    }

    #[test]
    fn modes_and_paths() {
        let Command::Run(req) = parse(args(&["--extract-to", r"C:\a.zip", r"C:\b.zip"])).unwrap()
        else {
            panic!()
        };
        assert_eq!(req.mode, Mode::ExtractTo);
        assert_eq!(
            req.paths,
            [PathBuf::from(r"C:\a.zip"), PathBuf::from(r"C:\b.zip")]
        );
        assert_eq!(req.password, None);
    }

    #[test]
    fn password_forms() {
        for form in [
            &["-p", "비밀", "x.zip"][..],
            &["--password", "비밀", "x.zip"],
            &["--password=비밀", "x.zip"],
        ] {
            let Command::Run(req) = parse(args(form)).unwrap() else {
                panic!()
            };
            assert_eq!(req.password.as_deref(), Some("비밀"));
            assert_eq!(req.mode, Mode::Open);
        }
    }

    #[test]
    fn conflicts_and_unknowns() {
        assert!(parse(args(&["--extract-here", "--test", "a.zip"])).is_err());
        assert!(parse(args(&["--bogus"])).is_err());
        assert!(parse(args(&["-p"])).is_err());
        assert_eq!(parse(args(&["--register"])).unwrap(), Command::Register);
        assert_eq!(parse(args(&["a.zip", "--help"])).unwrap(), Command::Help);
    }

    #[test]
    fn double_dash_allows_dash_names() {
        let Command::Run(req) = parse(args(&["--test", "--", "-이상한이름.zip"])).unwrap()
        else {
            panic!()
        };
        assert!(req.paths[0].ends_with("-이상한이름.zip"));
    }

    #[test]
    fn relative_paths_become_absolute() {
        let Command::Run(req) = parse(args(&["--test", "상대.zip"])).unwrap() else {
            panic!()
        };
        assert!(req.paths[0].is_absolute());
    }

    #[test]
    fn mode_names_roundtrip() {
        for name in [
            "open",
            "extract-here",
            "extract-to",
            "extract",
            "test",
            "compress",
            "compress-zip",
            "compress-tgz",
        ] {
            assert_eq!(Mode::parse(name).unwrap().as_str(), name);
        }
    }
}
