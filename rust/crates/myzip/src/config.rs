//! 사용자 설정.
//!
//! Python 판(QSettings)과 같은 자리에 같은 모양으로 저장한다. 그래야 기존 사용자의
//! 설정이 그대로 이어진다.
//!
//!   HKCU\Software\MyZIP\MyZIP\extract   conflict = "rename"   (REG_SZ)
//!                                         open_after = "true"   (REG_SZ)
//!
//! QSettings 는 bool 을 "true"/"false" 문자열로, 정수를 문자열이나 DWORD 로 쓴다.
//! 읽을 때는 둘 다 받아 준다.

use winreg::RegValue;
use winreg::enums::{KEY_READ, RegType};

use crate::regpath;

pub const SETTINGS_KEY: &str = r"Software\MyZIP\MyZIP";

/// 키 -> 기본값. Python 판 config.DEFAULTS 와 같다.
const DEFAULTS: &[(&str, &str)] = &[
    ("compress/format", "zip"),
    ("compress/level", "6"),
    ("compress/last_dir", ""),
    ("extract/mode", "subfolder"),  // here | subfolder | ask
    ("extract/conflict", "rename"), // overwrite | skip | rename | ask
    ("extract/last_dir", ""),
    ("extract/open_after", "true"),
    ("extract/strip_redundant", "true"),
    ("view/encoding", "auto"),
    ("general/confirm_close", "false"),
    ("general/theme", "auto"),
];

fn split(key: &str) -> (&str, &str) {
    key.rsplit_once('/').unwrap_or(("", key))
}

fn subkey(group: &str) -> String {
    let base = regpath::path(SETTINGS_KEY);
    if group.is_empty() {
        base
    } else {
        format!("{base}\\{}", group.replace('/', "\\"))
    }
}

fn default_of(key: &str) -> Option<&'static str> {
    DEFAULTS.iter().find(|(k, _)| *k == key).map(|(_, v)| *v)
}

/// REG_SZ 든 REG_DWORD 든 문자열로 읽는다.
fn decode(value: &RegValue) -> Option<String> {
    match value.vtype {
        RegType::REG_SZ | RegType::REG_EXPAND_SZ => {
            let wide: Vec<u16> = value
                .bytes
                .as_chunks::<2>()
                .0
                .iter()
                .map(|&pair| u16::from_le_bytes(pair))
                .collect();
            let end = wide.iter().position(|&c| c == 0).unwrap_or(wide.len());
            Some(String::from_utf16_lossy(&wide[..end]))
        }
        RegType::REG_DWORD if value.bytes.len() >= 4 => {
            Some(u32::from_le_bytes(value.bytes[..4].try_into().ok()?).to_string())
        }
        _ => None,
    }
}

pub fn get(key: &str) -> String {
    let (group, name) = split(key);
    regpath::hkcu()
        .open_subkey_with_flags(subkey(group), KEY_READ)
        .ok()
        .and_then(|k| k.get_raw_value(name).ok())
        .and_then(|v| decode(&v))
        .or_else(|| default_of(key).map(str::to_owned))
        .unwrap_or_default()
}

pub fn get_bool(key: &str) -> bool {
    matches!(
        get(key).trim().to_lowercase().as_str(),
        "true" | "1" | "yes"
    )
}

pub fn get_int(key: &str) -> i32 {
    get(key)
        .trim()
        .parse()
        .ok()
        .or_else(|| default_of(key).and_then(|d| d.parse().ok()))
        .unwrap_or(0)
}

pub fn set(key: &str, value: &str) -> std::io::Result<()> {
    let (group, name) = split(key);
    let (k, _) = regpath::hkcu().create_subkey(subkey(group))?;
    k.set_value(name, &value)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decodes_both_forms() {
        let sz = RegValue {
            vtype: RegType::REG_SZ,
            bytes: "rename\0"
                .encode_utf16()
                .flat_map(u16::to_le_bytes)
                .collect(),
        };
        assert_eq!(decode(&sz).as_deref(), Some("rename"));
        let dword = RegValue {
            vtype: RegType::REG_DWORD,
            bytes: 9u32.to_le_bytes().to_vec().into(),
        };
        assert_eq!(decode(&dword).as_deref(), Some("9"));
    }

    #[test]
    fn defaults_exist() {
        assert_eq!(default_of("extract/conflict"), Some("rename"));
        assert_eq!(split("extract/open_after"), ("extract", "open_after"));
    }
}
