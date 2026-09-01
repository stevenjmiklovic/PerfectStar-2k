//! Minimal deterministic ZIP writer for the stored (uncompressed) entries
//! needed by DOCX and EPUB. No external converter or runtime dependency.

use std::io;

pub struct Entry<'a> {
    pub name: &'a str,
    pub data: &'a [u8],
}

pub fn archive(entries: &[Entry<'_>]) -> io::Result<Vec<u8>> {
    if entries.len() > u16::MAX as usize {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "too many ZIP entries",
        ));
    }
    let mut out = Vec::new();
    let mut directory = Vec::new();

    for entry in entries {
        let name = entry.name.as_bytes();
        let name_len = u16::try_from(name.len())
            .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "ZIP entry name too long"))?;
        let size = u32::try_from(entry.data.len())
            .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "ZIP entry exceeds 4 GiB"))?;
        let offset = u32::try_from(out.len()).map_err(|_| {
            io::Error::new(io::ErrorKind::InvalidInput, "ZIP archive exceeds 4 GiB")
        })?;
        let crc = crc32(entry.data);

        u32le(&mut out, 0x0403_4b50);
        u16le(&mut out, 20); // version needed
        u16le(&mut out, 0x0800); // UTF-8 names
        u16le(&mut out, 0); // stored
        u16le(&mut out, 0); // deterministic time
        u16le(&mut out, 0x0021); // 1980-01-01
        u32le(&mut out, crc);
        u32le(&mut out, size);
        u32le(&mut out, size);
        u16le(&mut out, name_len);
        u16le(&mut out, 0);
        out.extend_from_slice(name);
        out.extend_from_slice(entry.data);

        u32le(&mut directory, 0x0201_4b50);
        u16le(&mut directory, 20);
        u16le(&mut directory, 20);
        u16le(&mut directory, 0x0800);
        u16le(&mut directory, 0);
        u16le(&mut directory, 0);
        u16le(&mut directory, 0x0021);
        u32le(&mut directory, crc);
        u32le(&mut directory, size);
        u32le(&mut directory, size);
        u16le(&mut directory, name_len);
        u16le(&mut directory, 0);
        u16le(&mut directory, 0);
        u16le(&mut directory, 0);
        u16le(&mut directory, 0);
        u32le(&mut directory, 0);
        u32le(&mut directory, offset);
        directory.extend_from_slice(name);
    }

    let directory_offset = u32::try_from(out.len())
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "ZIP archive exceeds 4 GiB"))?;
    let directory_size = u32::try_from(directory.len())
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "ZIP directory exceeds 4 GiB"))?;
    out.extend_from_slice(&directory);
    u32le(&mut out, 0x0605_4b50);
    u16le(&mut out, 0);
    u16le(&mut out, 0);
    u16le(&mut out, entries.len() as u16);
    u16le(&mut out, entries.len() as u16);
    u32le(&mut out, directory_size);
    u32le(&mut out, directory_offset);
    u16le(&mut out, 0);
    Ok(out)
}

fn crc32(bytes: &[u8]) -> u32 {
    let mut crc = 0xffff_ffffu32;
    for &byte in bytes {
        crc ^= u32::from(byte);
        for _ in 0..8 {
            crc = (crc >> 1) ^ (0xedb8_8320 & (0u32.wrapping_sub(crc & 1)));
        }
    }
    !crc
}

fn u16le(out: &mut Vec<u8>, value: u16) {
    out.extend_from_slice(&value.to_le_bytes());
}
fn u32le(out: &mut Vec<u8>, value: u32) {
    out.extend_from_slice(&value.to_le_bytes());
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_crc_is_standard_value() {
        assert_eq!(crc32(b""), 0);
        assert_eq!(crc32(b"123456789"), 0xcbf4_3926);
    }

    #[test]
    fn archive_has_zip_signatures() {
        let bytes = archive(&[Entry {
            name: "a.txt",
            data: b"hello",
        }])
        .unwrap();
        assert!(bytes.starts_with(b"PK\x03\x04"));
        assert!(bytes.windows(4).any(|w| w == b"PK\x01\x02"));
        assert!(bytes.ends_with(b"\0\0"));
    }
}
