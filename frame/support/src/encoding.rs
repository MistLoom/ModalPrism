//! 文本字节流编码自适应解码（UTF-8 / UTF-16 / GBK）。

/// 将字节流解码为 Rust 字符串，按 BOM 与启发式选择编码。
pub fn decode_text_bytes(bytes: &[u8]) -> String {
    if bytes.is_empty() {
        return String::new();
    }

    // UTF-8 BOM
    let bytes = if bytes.starts_with(&[0xEF, 0xBB, 0xBF]) {
        &bytes[3..]
    } else {
        bytes
    };

    // UTF-16 LE BOM（PowerShell 默认 stdout 常见）
    if bytes.len() >= 2 && bytes[0] == 0xFF && bytes[1] == 0xFE {
        return decode_utf16(&bytes[2..], true);
    }
    // UTF-16 BE BOM
    if bytes.len() >= 2 && bytes[0] == 0xFE && bytes[1] == 0xFF {
        return decode_utf16(&bytes[2..], false);
    }

    if std::str::from_utf8(bytes).is_ok() {
        return std::str::from_utf8(bytes)
            .expect("utf-8 checked")
            .to_string();
    }

    // 无 BOM 的 UTF-16 LE（ASCII 文本在 PowerShell 中常为此形态）
    if looks_like_utf16_le(bytes) {
        if let Some(text) = try_decode_utf16(bytes, true) {
            return text;
        }
    }

    #[cfg(windows)]
    {
        use encoding_rs::GBK;
        let (decoded, _, had_errors) = GBK.decode(bytes);
        if !had_errors {
            return decoded.into_owned();
        }
    }

    String::from_utf8_lossy(bytes).into_owned()
}

/// 严格文本解码：拒绝用 U+FFFD 替换非法字节（避免二进制经 read→write 静默损坏）。
pub fn decode_text_bytes_strict(bytes: &[u8]) -> Result<String, String> {
    if bytes.is_empty() {
        return Ok(String::new());
    }

    let bytes = if bytes.starts_with(&[0xEF, 0xBB, 0xBF]) {
        &bytes[3..]
    } else {
        bytes
    };

    if bytes.len() >= 2 && bytes[0] == 0xFF && bytes[1] == 0xFE {
        return try_decode_utf16(&bytes[2..], true).ok_or_else(|| "UTF-16 LE 解码失败".into());
    }
    if bytes.len() >= 2 && bytes[0] == 0xFE && bytes[1] == 0xFF {
        return try_decode_utf16(&bytes[2..], false).ok_or_else(|| "UTF-16 BE 解码失败".into());
    }

    if std::str::from_utf8(bytes).is_ok() {
        return Ok(std::str::from_utf8(bytes)
            .expect("utf-8 checked")
            .to_string());
    }

    if looks_like_utf16_le(bytes) {
        if let Some(text) = try_decode_utf16(bytes, true) {
            return Ok(text);
        }
    }

    #[cfg(windows)]
    {
        use encoding_rs::GBK;
        let (decoded, _, had_errors) = GBK.decode(bytes);
        if !had_errors {
            return Ok(decoded.into_owned());
        }
    }

    Err("含非文本字节，无法作为文本读取（二进制请使用 base64 写入能力）".into())
}

fn decode_utf16(bytes: &[u8], le: bool) -> String {
    try_decode_utf16(bytes, le).unwrap_or_else(|| String::from_utf8_lossy(bytes).into_owned())
}

fn try_decode_utf16(bytes: &[u8], le: bool) -> Option<String> {
    if bytes.len() % 2 != 0 {
        return None;
    }
    let units: Vec<u16> = bytes
        .chunks_exact(2)
        .map(|c| {
            if le {
                u16::from_le_bytes([c[0], c[1]])
            } else {
                u16::from_be_bytes([c[0], c[1]])
            }
        })
        .collect();
    String::from_utf16(&units).ok()
}

/// 检测无 BOM 的 UTF-16 LE：奇数位大量为 0x00。
fn looks_like_utf16_le(bytes: &[u8]) -> bool {
    if bytes.len() < 4 || bytes.len() % 2 != 0 {
        return false;
    }
    let odd_nulls = bytes.iter().skip(1).step_by(2).filter(|&&b| b == 0).count();
    odd_nulls * 3 >= bytes.len() / 2
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn utf8_chinese_roundtrip() {
        let text = "Ability 运行时：同一套能力";
        assert_eq!(decode_text_bytes(text.as_bytes()), text);
    }

    #[test]
    fn utf16_le_bom() {
        let text = "Ability 运行时";
        let mut bytes = vec![0xFF, 0xFE];
        for u in text.encode_utf16() {
            bytes.extend_from_slice(&u.to_le_bytes());
        }
        assert_eq!(decode_text_bytes(&bytes), text);
    }

    #[test]
    fn strict_rejects_lossy_binary() {
        let bytes = [0x48, 0x49, 0x00, 0x00, 0x0a, 0x0d, 0xff, 0xfe, 0x42];
        assert!(decode_text_bytes_strict(&bytes).is_err());
    }

    #[cfg(windows)]
    #[test]
    fn gbk_chinese() {
        use encoding_rs::GBK;
        let (encoded, _, _) = GBK.encode("中文测试");
        assert_eq!(decode_text_bytes(&encoded), "中文测试");
    }
}
