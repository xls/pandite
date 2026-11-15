//! The Windows resources of the executable, written as a compiled
//! resource file (`.res`) in pure Rust: the program icon, the version
//! information and the manifest.
//!
//! The build script includes this file with `#[path]`; the library
//! compiles it for its unit tests only.

/// Resource type of one icon image.
pub const RT_ICON: u16 = 3;
/// Resource type of the icon directory.
pub const RT_GROUP_ICON: u16 = 14;
/// Resource type of the version information.
pub const RT_VERSION: u16 = 16;
/// Resource type of the application manifest.
pub const RT_MANIFEST: u16 = 24;
/// English (United States).
pub const LANG_EN_US: u16 = 0x0409;
/// The Unicode code page, paired with [`LANG_EN_US`] in the version
/// strings.
pub const CODE_PAGE_UNICODE: u16 = 0x04B0;

/// The strings of the version resource.
pub struct VersionStrings<'a> {
    /// `major.minor.patch`, as Cargo writes the package version.
    pub version: &'a str,
    /// Name pairs in the order they are written.
    pub fields: &'a [(&'a str, &'a str)],
}

/// One resource entry: 32-byte header with ordinal type and name, then the
/// data padded to a 4-byte boundary.
fn push_resource(out: &mut Vec<u8>, kind: u16, id: u16, flags: u16, lang: u16, data: &[u8]) {
    let size = u32::try_from(data.len()).expect("resource fits in 4 GiB");
    out.extend_from_slice(&size.to_le_bytes());
    out.extend_from_slice(&32u32.to_le_bytes());
    out.extend_from_slice(&0xFFFFu16.to_le_bytes());
    out.extend_from_slice(&kind.to_le_bytes());
    out.extend_from_slice(&0xFFFFu16.to_le_bytes());
    out.extend_from_slice(&id.to_le_bytes());
    out.extend_from_slice(&0u32.to_le_bytes()); // data version
    out.extend_from_slice(&flags.to_le_bytes());
    out.extend_from_slice(&lang.to_le_bytes());
    out.extend_from_slice(&0u32.to_le_bytes()); // version
    out.extend_from_slice(&0u32.to_le_bytes()); // characteristics
    out.extend_from_slice(data);
    pad4(out);
}

fn pad4(out: &mut Vec<u8>) {
    while !out.len().is_multiple_of(4) {
        out.push(0);
    }
}

fn u16_at(b: &[u8], at: usize) -> Option<u16> {
    Some(u16::from_le_bytes([*b.get(at)?, *b.get(at + 1)?]))
}

fn u32_at(b: &[u8], at: usize) -> Option<u32> {
    Some(u32::from_le_bytes([
        *b.get(at)?,
        *b.get(at + 1)?,
        *b.get(at + 2)?,
        *b.get(at + 3)?,
    ]))
}

fn utf16z(text: &str, out: &mut Vec<u8>) {
    for unit in text.encode_utf16().chain(std::iter::once(0)) {
        out.extend_from_slice(&unit.to_le_bytes());
    }
}

/// The value of a version block.
enum Value<'a> {
    None,
    Binary(&'a [u8]),
    Text(&'a str),
}

/// One block of the version resource: `wLength`, `wValueLength`, `wType`,
/// the key as UTF-16 with its terminator, padding to 32 bits, the value,
/// then each child on a 32-bit boundary. `wLength` covers everything but
/// the padding after the last byte; `wValueLength` counts bytes for a
/// binary value and UTF-16 units (with the terminator) for text.
fn block(key: &str, value: Value<'_>, children: &[Vec<u8>]) -> Vec<u8> {
    let mut out = vec![0u8; 6];
    utf16z(key, &mut out);
    pad4(&mut out);
    let (value_len, kind) = match value {
        Value::None => (0, 1u16),
        Value::Binary(bytes) => {
            out.extend_from_slice(bytes);
            (bytes.len(), 0)
        }
        Value::Text(text) => {
            utf16z(text, &mut out);
            (text.encode_utf16().count() + 1, 1)
        }
    };
    for child in children {
        pad4(&mut out);
        out.extend_from_slice(child);
    }
    let len = u16::try_from(out.len()).expect("version block fits in 64 KiB");
    let value_len = u16::try_from(value_len).expect("version value fits in 64 KiB");
    out[0..2].copy_from_slice(&len.to_le_bytes());
    out[2..4].copy_from_slice(&value_len.to_le_bytes());
    out[4..6].copy_from_slice(&kind.to_le_bytes());
    out
}

/// `major.minor.patch[.build]` as the two 32-bit halves of a fixed
/// version number. Missing or unreadable parts count as 0.
pub fn version_words(version: &str) -> (u32, u32) {
    let mut parts = version
        .split(['.', '-', '+'])
        .map(|p| p.parse::<u16>().unwrap_or(0));
    let mut next = || u32::from(parts.next().unwrap_or(0));
    let (a, b, c, d) = (next(), next(), next(), next());
    ((a << 16) | b, (c << 16) | d)
}

/// The VS_VERSIONINFO block: the fixed file information (an application
/// for 32-bit Windows NT with `strings.version`), the strings in the
/// 040904B0 table, and the translation entry that names that table.
pub fn version_info(strings: &VersionStrings<'_>) -> Vec<u8> {
    let (ms, ls) = version_words(strings.version);
    let mut fixed = Vec::with_capacity(52);
    for word in [
        0xFEEF_04BD, // signature
        0x0001_0000, // structure version 1.0
        ms,          // file version
        ls,
        ms, // product version
        ls,
        0x3F,        // flags mask
        0,           // flags
        0x0004_0004, // VOS_NT_WINDOWS32
        1,           // VFT_APP
        0,           // subtype
        0,           // date
        0,
    ] {
        fixed.extend_from_slice(&u32::to_le_bytes(word));
    }
    let entries: Vec<Vec<u8>> = strings
        .fields
        .iter()
        .map(|(name, text)| block(name, Value::Text(text), &[]))
        .collect();
    let table_key = format!("{LANG_EN_US:04X}{CODE_PAGE_UNICODE:04X}");
    let table = block(&table_key, Value::None, &entries);
    let string_info = block("StringFileInfo", Value::None, &[table]);
    let mut translation = Vec::new();
    translation.extend_from_slice(&LANG_EN_US.to_le_bytes());
    translation.extend_from_slice(&CODE_PAGE_UNICODE.to_le_bytes());
    let var = block("Translation", Value::Binary(&translation), &[]);
    let var_info = block("VarFileInfo", Value::None, &[var]);
    block(
        "VS_VERSION_INFO",
        Value::Binary(&fixed),
        &[string_info, var_info],
    )
}

/// Turn an `.ico` file into RT_ICON images plus the RT_GROUP_ICON
/// directory that names them, and add the version block and the manifest.
/// Returns `None` when the icon file is malformed.
pub fn build_res(ico: &[u8], version: &[u8], manifest: &[u8]) -> Option<Vec<u8>> {
    if u16_at(ico, 0)? != 0 || u16_at(ico, 2)? != 1 {
        return None;
    }
    let count = u16_at(ico, 4)?;
    let mut res = Vec::new();
    // The empty entry every .res file starts with.
    push_resource(&mut res, 0, 0, 0, 0, &[]);
    let mut group = Vec::new();
    group.extend_from_slice(&0u16.to_le_bytes());
    group.extend_from_slice(&1u16.to_le_bytes());
    group.extend_from_slice(&count.to_le_bytes());
    for i in 0..usize::from(count) {
        let entry = 6 + 16 * i;
        // Width, height, color count, reserved, planes and bit count are
        // the same in the file directory and the group directory.
        let head = ico.get(entry..entry + 8)?;
        let len = u32_at(ico, entry + 8)? as usize;
        let offset = u32_at(ico, entry + 12)? as usize;
        let image = ico.get(offset..offset.checked_add(len)?)?;
        let id = u16::try_from(i + 1).ok()?;
        // MOVEABLE | DISCARDABLE, as resource compilers write icons.
        push_resource(&mut res, RT_ICON, id, 0x1010, LANG_EN_US, image);
        group.extend_from_slice(head);
        group.extend_from_slice(&u32::try_from(len).ok()?.to_le_bytes());
        group.extend_from_slice(&id.to_le_bytes());
    }
    push_resource(&mut res, RT_GROUP_ICON, 1, 0x1030, LANG_EN_US, &group);
    // MOVEABLE | PURE, as resource compilers write version blocks.
    push_resource(&mut res, RT_VERSION, 1, 0x0030, LANG_EN_US, version);
    push_resource(&mut res, RT_MANIFEST, 1, 0x0030, LANG_EN_US, manifest);
    Some(res)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A parsed version block.
    #[derive(Debug)]
    struct Node {
        key: String,
        kind: u16,
        value: Vec<u8>,
        text: Option<String>,
        children: Vec<Node>,
    }

    fn utf16_at(b: &[u8], at: usize, units: usize) -> String {
        let v: Vec<u16> = (0..units).map(|i| u16_at(b, at + 2 * i).unwrap()).collect();
        String::from_utf16(&v).unwrap()
    }

    /// Parse the block at `at`, checking every length field.
    fn parse(b: &[u8], at: usize) -> Node {
        assert_eq!(at % 4, 0, "block at {at} starts on a 32-bit boundary");
        let len = usize::from(u16_at(b, at).unwrap());
        let value_len = usize::from(u16_at(b, at + 2).unwrap());
        let kind = u16_at(b, at + 4).unwrap();
        assert!(kind <= 1, "wType is 0 or 1");
        let end = at + len;
        assert!(end <= b.len(), "block at {at} fits");
        let mut p = at + 6;
        let mut units = 0;
        while u16_at(b, p + 2 * units).unwrap() != 0 {
            units += 1;
        }
        let key = utf16_at(b, p, units);
        p += 2 * (units + 1);
        p = p.next_multiple_of(4);
        let (value, text) = if kind == 1 && value_len > 0 {
            let s = utf16_at(b, p, value_len - 1);
            assert_eq!(
                u16_at(b, p + 2 * (value_len - 1)),
                Some(0),
                "{key} ends with 0"
            );
            let bytes = b[p..p + 2 * value_len].to_vec();
            p += 2 * value_len;
            (bytes, Some(s))
        } else {
            let bytes = b[p..p + value_len].to_vec();
            p += value_len;
            (bytes, None)
        };
        let mut children = Vec::new();
        while p.next_multiple_of(4) < end {
            p = p.next_multiple_of(4);
            let child = parse(b, p);
            p += usize::from(u16_at(b, p).unwrap());
            children.push(child);
        }
        assert_eq!(p, end, "wLength of {key} ends at its last byte");
        Node {
            key,
            kind,
            value,
            text,
            children,
        }
    }

    const FIELDS: &[(&str, &str)] = &[
        ("CompanyName", "Pandite"),
        ("FileDescription", "Pandite"),
        ("FileVersion", "1.22.333"),
        ("InternalName", "pandite"),
        ("LegalCopyright", "MIT License"),
        ("OriginalFilename", "pandite.exe"),
        ("ProductName", "Pandite"),
        ("ProductVersion", "1.22.333"),
    ];

    #[test]
    fn version_block_parses_back() {
        let bytes = version_info(&VersionStrings {
            version: "1.22.333",
            fields: FIELDS,
        });
        let root = parse(&bytes, 0);
        assert_eq!(usize::from(u16_at(&bytes, 0).unwrap()), bytes.len());
        assert_eq!(root.key, "VS_VERSION_INFO");
        assert_eq!(root.kind, 0);
        assert_eq!(root.value.len(), 52);
        let word = |i: usize| u32_at(&root.value, 4 * i).unwrap();
        assert_eq!(word(0), 0xFEEF_04BD, "signature");
        assert_eq!(
            (word(2), word(3)),
            (1 << 16 | 22, 333 << 16),
            "file version"
        );
        assert_eq!(
            (word(4), word(5)),
            (1 << 16 | 22, 333 << 16),
            "product version"
        );
        assert_eq!(word(8), 0x0004_0004, "Windows NT");
        assert_eq!(word(9), 1, "an application");
        assert_eq!(root.children.len(), 2);

        let strings = &root.children[0];
        assert_eq!((strings.key.as_str(), strings.kind), ("StringFileInfo", 1));
        let table = &strings.children[0];
        assert_eq!(table.key, "040904B0");
        let got: Vec<(String, String)> = table
            .children
            .iter()
            .map(|c| (c.key.clone(), c.text.clone().unwrap()))
            .collect();
        let want: Vec<(String, String)> = FIELDS
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect();
        assert_eq!(got, want);

        let vars = &root.children[1];
        assert_eq!(vars.key, "VarFileInfo");
        let translation = &vars.children[0];
        assert_eq!(
            (translation.key.as_str(), translation.kind),
            ("Translation", 0)
        );
        assert_eq!(translation.value, vec![0x09, 0x04, 0xB0, 0x04]);
    }

    #[test]
    fn version_words_read_cargo_versions() {
        assert_eq!(version_words("0.1.0"), (1, 0));
        assert_eq!(version_words("2.3.4"), (2 << 16 | 3, 4 << 16));
        assert_eq!(version_words("1.2.3-beta.1"), (1 << 16 | 2, 3 << 16));
        assert_eq!(version_words("x"), (0, 0));
    }

    #[test]
    fn resource_file_lists_icons_version_and_manifest() {
        let ico = include_bytes!("../assets/pandite.ico").to_vec();
        let version = version_info(&VersionStrings {
            version: "0.1.0",
            fields: FIELDS,
        });
        let res = build_res(&ico, &version, b"<assembly/>").unwrap();
        // Walk the entries: (type, id, data length).
        let mut seen = Vec::new();
        let mut at = 0;
        while at < res.len() {
            let size = u32_at(&res, at).unwrap() as usize;
            let header = u32_at(&res, at + 4).unwrap() as usize;
            assert_eq!(header, 32);
            let kind = u16_at(&res, at + 10).unwrap();
            let id = u16_at(&res, at + 14).unwrap();
            seen.push((kind, id, size));
            at = (at + header + size).next_multiple_of(4);
        }
        assert_eq!(at, res.len());
        let count = crate::icon::ICO_SIZES.len();
        assert_eq!(seen.len(), 1 + count + 3);
        assert!(seen[1..=count].iter().all(|e| e.0 == RT_ICON));
        assert_eq!(seen[count + 1].0, RT_GROUP_ICON);
        assert_eq!(seen[count + 1].2, 6 + 14 * count, "group directory size");
        assert_eq!((seen[count + 2].0, seen[count + 2].1), (RT_VERSION, 1));
        assert_eq!(seen[count + 2].2, version.len());
        assert_eq!(seen[count + 3].0, RT_MANIFEST);
        // A malformed icon file is refused.
        assert!(build_res(&ico[..10], &version, b"").is_none());
    }
}
