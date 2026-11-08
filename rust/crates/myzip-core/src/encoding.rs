//! 아카이브 내부 파일명 인코딩 자동 판별.
//!
//! ZIP 은 UTF-8 플래그(bit 11)가 없으면 파일명 바이트의 인코딩이 명시되지
//! 않는다. 한국에서 만들어진 대부분의 ZIP 은 CP949(EUC-KR 확장), 일본은
//! CP932, 중국은 GBK 로 기록되어 있어서 그냥 읽으면 이름이 깨진다.
//! 여기서는 원본 바이트를 후보 인코딩으로 디코딩해 보고 점수를 매겨
//! 가장 그럴듯한 것을 고른다.
//!
//! 인코딩 이름은 Python 판과 같은 문자열("utf-8", "cp949" ...)을 쓴다.
//! 설정 파일과 UI 가 이 이름을 그대로 주고받는다.

use encoding_rs::{BIG5, EUC_KR, Encoding, GBK, SHIFT_JIS, WINDOWS_1252};
use unicode_general_category::{GeneralCategory, get_general_category};

/// 우선순위 순서. 한국어 환경이므로 CP949 를 앞에 둔다.
pub const CANDIDATES: &[&str] = &["utf-8", "cp949", "cp932", "gbk", "big5", "cp1252"];

/// UI 의 인코딩 선택 목록에 보이는 이름.
pub const DISPLAY_NAMES: &[(&str, &str)] = &[
    ("utf-8", "UTF-8"),
    ("cp949", "한국어 (CP949)"),
    ("cp932", "일본어 (CP932)"),
    ("gbk", "중국어 간체 (GBK)"),
    ("big5", "중국어 번체 (Big5)"),
    ("cp1252", "서유럽 (CP1252)"),
    ("cp437", "DOS (CP437)"),
];

pub fn display_name(encoding: &str) -> &str {
    DISPLAY_NAMES
        .iter()
        .find(|(id, _)| *id == encoding)
        .map(|(_, name)| *name)
        .unwrap_or(encoding)
}

fn whatwg(encoding: &str) -> Option<&'static Encoding> {
    // WHATWG 의 EUC-KR 은 실제로는 CP949(UHC) 이고, Shift_JIS 는 CP932 를 포함한다.
    match encoding {
        "cp949" | "euc-kr" | "uhc" => Some(EUC_KR),
        "cp932" | "shift_jis" | "sjis" => Some(SHIFT_JIS),
        "gbk" | "gb2312" | "cp936" => Some(GBK),
        "big5" | "cp950" => Some(BIG5),
        "cp1252" | "latin-1" => Some(WINDOWS_1252),
        _ => None,
    }
}

/// 엄격 디코딩. 해당 인코딩으로 풀리지 않는 바이트가 있으면 None.
pub fn decode_strict(raw: &[u8], encoding: &str) -> Option<String> {
    match encoding {
        "utf-8" | "utf8" => std::str::from_utf8(raw).ok().map(str::to_owned),
        "cp437" => Some(decode_cp437(raw)),
        "ascii" => raw
            .is_ascii()
            .then(|| String::from_utf8_lossy(raw).into_owned()),
        other => whatwg(other)?
            .decode_without_bom_handling_and_without_replacement(raw)
            .map(|cow| cow.into_owned()),
    }
}

/// CP437 은 0x00~0xFF 전체가 문자에 1:1 대응하므로 실패하지 않는다.
pub fn decode_cp437(raw: &[u8]) -> String {
    raw.iter()
        .map(|&b| {
            if b < 0x80 {
                b as char
            } else {
                CP437_HIGH[(b - 0x80) as usize]
            }
        })
        .collect()
}

#[rustfmt::skip]
const CP437_HIGH: [char; 128] = [
    'Ç','ü','é','â','ä','à','å','ç','ê','ë','è','ï','î','ì','Ä','Å',
    'É','æ','Æ','ô','ö','ò','û','ù','ÿ','Ö','Ü','¢','£','¥','₧','ƒ',
    'á','í','ó','ú','ñ','Ñ','ª','º','¿','⌐','¬','½','¼','¡','«','»',
    '░','▒','▓','│','┤','╡','╢','╖','╕','╣','║','╗','╝','╜','╛','┐',
    '└','┴','┬','├','─','┼','╞','╟','╚','╔','╩','╦','╠','═','╬','╧',
    '╨','╤','╥','╙','╘','╒','╓','╫','╪','┘','┌','█','▄','▌','▐','▀',
    'α','ß','Γ','π','Σ','σ','µ','τ','Φ','Θ','Ω','δ','∞','φ','ε','∩',
    '≡','±','≥','≤','⌠','⌡','÷','≈','°','∙','·','√','ⁿ','²','■','\u{a0}',
];

/// 한글 / 한자 / 가나 영역.
const CJK_RANGES: &[(u32, u32)] = &[
    (0xAC00, 0xD7A3), // 한글 음절
    (0x1100, 0x11FF), // 한글 자모
    (0x3130, 0x318F), // 호환 자모
    (0x4E00, 0x9FFF), // 한중일 통합 한자
    (0x3040, 0x309F), // 히라가나
    (0x30A0, 0x30FF), // 가타카나
    (0xFF01, 0xFF60), // 전각
];

/// 잘못된 인코딩으로 풀었을 때 흔히 나오는 쓰레기 문자들.
const MOJIBAKE: &str =
    "ÀÁÂÃÄÅÆÇÈÉÊËÌÍÎÏÐÑÒÓÔÕÖ×ØÙÚÛÜÝÞßàáâãäåæçèéêëìíîïðñòóôõö÷øùúûüýþÿ¿½¾¼±¶§¨©ª«¬®¯";

const FILENAME_PUNCT: &str = " ._-()[]{}#&+,'!~@$%^=;`";

fn is_cjk(ch: char) -> bool {
    let cp = ch as u32;
    CJK_RANGES.iter().any(|&(lo, hi)| (lo..=hi).contains(&cp))
}

/// 파일명에 나올 리 없는 제어문자. 탭과 줄바꿈은 아래에서 따로 감점된다.
fn is_bad_control(ch: char) -> bool {
    matches!(ch, '\x00'..='\x08' | '\x0b' | '\x0c' | '\x0e'..='\x1f' | '\x7f')
}

/// 디코딩 결과가 '진짜 파일명'처럼 보이는 정도를 점수로 환산한다.
pub fn score_decoded(text: &str) -> f64 {
    let mut count = 0usize;
    let mut score = 0.0;
    for ch in text.chars() {
        count += 1;
        score += if is_bad_control(ch) {
            -50.0
        } else if MOJIBAKE.contains(ch) {
            // 깨진 문자열의 전형적인 흔적
            -4.0
        } else if is_cjk(ch) {
            3.0
        } else if ch.is_alphanumeric() || FILENAME_PUNCT.contains(ch) {
            1.0
        } else if matches!(ch, '\\' | '/' | ':') {
            0.5
        } else {
            match get_general_category(ch) {
                // 미할당 / 사용자영역 / 서로게이트
                GeneralCategory::Unassigned
                | GeneralCategory::PrivateUse
                | GeneralCategory::Surrogate => -20.0,
                GeneralCategory::Control | GeneralCategory::Format => -10.0,
                _ => 0.2,
            }
        };
    }
    if count == 0 {
        0.0
    } else {
        score / count as f64
    }
}

/// 원본 바이트를 가장 그럴듯한 인코딩으로 디코딩한다.
///
/// (디코딩된 문자열, 사용한 인코딩 이름) 을 돌려준다.
pub fn decode_best(raw: &[u8]) -> (String, &'static str) {
    if raw.is_empty() {
        return (String::new(), "utf-8");
    }
    // ASCII 뿐이면 어떤 인코딩이든 결과가 같다.
    if raw.is_ascii() {
        return (String::from_utf8_lossy(raw).into_owned(), "utf-8");
    }

    let mut best: Option<(String, &'static str, f64)> = None;
    for &enc in CANDIDATES {
        let Some(text) = decode_strict(raw, enc) else {
            continue;
        };
        let mut s = score_decoded(&text);
        // UTF-8 로 성공적으로 풀렸다면 우연일 확률이 매우 낮으므로 가산점
        if enc == "utf-8" {
            s += 1.5;
        }
        if best.as_ref().is_none_or(|(_, _, b)| s > *b) {
            best = Some((text, enc, s));
        }
    }
    match best {
        Some((text, enc, _)) => (text, enc),
        None => (decode_cp437(raw), "cp437"),
    }
}

/// 지정한 인코딩으로 풀고, 안 되면 가장 그럴듯한 것으로 푼다.
pub fn decode_with(raw: &[u8], encoding: &str) -> String {
    decode_strict(raw, encoding).unwrap_or_else(|| decode_best(raw).0)
}

/// 아카이브 전체 파일명을 한꺼번에 보고 인코딩 하나를 결정한다.
///
/// 파일별로 따로 판별하면 같은 아카이브 안에서 인코딩이 뒤섞일 수 있으므로
/// 전체 합산 점수로 하나를 고른다.
pub fn detect_archive_encoding<'a, I>(raw_names: I) -> &'static str
where
    I: IntoIterator<Item = &'a [u8]>,
{
    let non_ascii: Vec<&[u8]> = raw_names.into_iter().filter(|r| !r.is_ascii()).collect();
    if non_ascii.is_empty() {
        return "utf-8";
    }

    let mut best: Option<(&'static str, f64)> = None;
    'candidates: for &enc in CANDIDATES {
        let mut total = 0.0;
        for raw in &non_ascii {
            match decode_strict(raw, enc) {
                Some(text) => total += score_decoded(&text),
                None => continue 'candidates,
            }
        }
        if enc == "utf-8" {
            total += 1.5 * non_ascii.len() as f64;
        }
        // 점수가 같으면 앞선(우선순위 높은) 후보를 남긴다.
        if best.is_none_or(|(_, b)| total > b) {
            best = Some((enc, total));
        }
    }
    best.map_or("cp437", |(enc, _)| enc)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cp949_names_are_detected() {
        let names: Vec<Vec<u8>> = ["한글파일.txt", "문서/보고서.hwp"]
            .iter()
            .map(|s| EUC_KR.encode(s).0.into_owned())
            .collect();
        let enc = detect_archive_encoding(names.iter().map(Vec::as_slice));
        assert_eq!(enc, "cp949");
        assert_eq!(decode_with(&names[0], enc), "한글파일.txt");
    }

    #[test]
    fn utf8_wins_when_valid() {
        let raw = "보고서 2026.txt".as_bytes();
        assert_eq!(decode_best(raw), ("보고서 2026.txt".to_string(), "utf-8"));
    }

    #[test]
    fn ascii_is_utf8() {
        assert_eq!(detect_archive_encoding([b"readme.txt".as_slice()]), "utf-8");
    }

    #[test]
    fn cp437_roundtrip_high_bytes() {
        assert_eq!(decode_cp437(&[0x80, 0xE1, 0xFF]), "Çß\u{a0}");
    }
}
