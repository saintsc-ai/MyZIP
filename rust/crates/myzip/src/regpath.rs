//! HKCU 레지스트리 경로.
//!
//! 모든 키는 HKEY_CURRENT_USER 아래에만 쓴다. 관리자 권한이 필요 없고,
//! 제거할 때도 우리가 만든 키만 지우면 된다.
//!
//! 환경 변수 `MYZIP_REG_ROOT` 를 주면 모든 경로를 그 아래로 옮긴다.
//! 테스트가 사용자의 실제 탐색기 설정을 건드리지 않게 하기 위한 장치다.

use winreg::RegKey;
use winreg::enums::HKEY_CURRENT_USER;

pub const ENV_ROOT: &str = "MYZIP_REG_ROOT";

/// 샌드박스 중이면 그 루트.
pub fn sandbox() -> Option<String> {
    std::env::var(ENV_ROOT)
        .ok()
        .filter(|r| !r.trim().is_empty())
}

/// HKCU 기준 경로를 (샌드박스를 반영해) 돌려준다.
pub fn path(relative: &str) -> String {
    match sandbox() {
        Some(root) => format!("{}\\{relative}", root.trim_end_matches('\\')),
        None => relative.to_owned(),
    }
}

pub fn hkcu() -> RegKey {
    RegKey::predef(HKEY_CURRENT_USER)
}
