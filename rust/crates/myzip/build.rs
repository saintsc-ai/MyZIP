//! exe 에 Windows 리소스(아이콘, 버전 정보, 매니페스트)를 넣는다.
//!
//! 아이콘을 전부 exe 안에 넣어 두면 배포 폴더에 .ico 파일을 따로 둘 필요가 없다.
//! 레지스트리에는 `"MyZIP.exe",-<ID>` 꼴로 적는다. 음수는 리소스 ID 라는 뜻이다.
//! ID 표는 여기 한 곳에서만 정하고, `icon_ids.rs` 로 만들어 registry.rs 가 읽는다.

use std::env;
use std::fmt::Write as _;
use std::fs;
use std::path::PathBuf;

/// (이름, 리소스 ID). 탐색기는 ID 가 가장 작은 아이콘을 exe 아이콘으로 쓴다.
const ICONS: &[(&str, u16)] = &[
    ("app", 1),
    ("zip", 2),
    ("tar", 3),
    ("tgz", 4),
    ("rar", 5),
    ("7z", 6),
    ("file", 7),
];

fn main() {
    let manifest_dir = PathBuf::from(env::var("CARGO_MANIFEST_DIR").unwrap());
    let out = PathBuf::from(env::var("OUT_DIR").unwrap());
    let icons_dir = manifest_dir.join("../../../resources/icons");

    let mut ids = String::from("/// build.rs 가 만든 아이콘 리소스 ID 표.\n");
    ids.push_str("pub const ICON_IDS: &[(&str, u16)] = &[\n");
    for (name, id) in ICONS {
        writeln!(ids, "    ({name:?}, {id}),").unwrap();
    }
    ids.push_str("];\n");
    fs::write(out.join("icon_ids.rs"), ids).unwrap();

    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed=app.manifest");
    if env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
        return;
    }

    let version = env::var("CARGO_PKG_VERSION").unwrap();
    let part = |key: &str| env::var(key).unwrap().parse::<u16>().unwrap_or(0);
    let (major, minor, patch) = (
        part("CARGO_PKG_VERSION_MAJOR"),
        part("CARGO_PKG_VERSION_MINOR"),
        part("CARGO_PKG_VERSION_PATCH"),
    );
    let escape = |p: PathBuf| p.display().to_string().replace('\\', "\\\\");

    // rc.exe 는 #pragma code_page(65001) 가 있으면 UTF-8 로 읽는다.
    let mut rc = String::from("#pragma code_page(65001)\n#include <winver.h>\n\n");
    for (name, id) in ICONS {
        let path = icons_dir.join(format!("{name}.ico"));
        assert!(path.is_file(), "아이콘이 없습니다: {}", path.display());
        println!("cargo:rerun-if-changed={}", path.display());
        let path = escape(dunce(path));
        writeln!(rc, "{id} ICON \"{path}\"").unwrap();
    }
    let manifest = escape(manifest_dir.join("app.manifest"));
    writeln!(rc, "\n1 24 \"{manifest}\"\n").unwrap();
    write!(
        rc,
        r#"VS_VERSION_INFO VERSIONINFO
FILEVERSION {major},{minor},{patch},0
PRODUCTVERSION {major},{minor},{patch},0
FILEFLAGSMASK 0x3fL
FILEFLAGS 0x0L
FILEOS VOS_NT_WINDOWS32
FILETYPE VFT_APP
FILESUBTYPE 0x0L
BEGIN
    BLOCK "StringFileInfo"
    BEGIN
        BLOCK "041204b0"
        BEGIN
            VALUE "CompanyName", "MyZIP"
            VALUE "FileDescription", "MyZIP 압축 프로그램"
            VALUE "FileVersion", "{version}"
            VALUE "InternalName", "MyZIP"
            VALUE "LegalCopyright", "MIT License"
            VALUE "OriginalFilename", "MyZIP.exe"
            VALUE "ProductName", "MyZIP"
            VALUE "ProductVersion", "{version}"
        END
    END
    BLOCK "VarFileInfo"
    BEGIN
        VALUE "Translation", 0x412, 1200
    END
END
"#
    )
    .unwrap();

    let rc_path = out.join("myzip.rc");
    fs::write(&rc_path, rc).unwrap();
    embed_resource::compile(&rc_path, embed_resource::NONE)
        .manifest_required()
        .unwrap();
}

/// `..` 를 걷어낸 절대 경로. rc.exe 는 `\\?\` 접두사를 읽지 못한다.
fn dunce(path: PathBuf) -> PathBuf {
    let Ok(canonical) = path.canonicalize() else {
        return path;
    };
    let text = canonical.display().to_string();
    match text.strip_prefix(r"\\?\") {
        Some(rest) => PathBuf::from(rest),
        None => canonical,
    }
}
