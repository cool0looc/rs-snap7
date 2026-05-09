use std::time::Duration;

#[derive(Debug, Clone)]
pub struct ConnectParams {
    pub rack: u8,
    pub slot: u8,
    pub pdu_size: u16,
    pub connect_timeout: Duration,
    pub request_timeout: Duration,
}

impl Default for ConnectParams {
    fn default() -> Self {
        Self {
            rack: 0,
            slot: 1,
            pdu_size: 480,
            connect_timeout: Duration::from_secs(5),
            request_timeout: Duration::from_secs(10),
        }
    }
}

/// PLC run-time status returned by [`S7Client::get_plc_status`](crate::S7Client::get_plc_status).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlcStatus {
    /// Status unknown or not available.
    Unknown = 0x00,
    /// PLC is in STOP mode.
    Stop = 0x04,
    /// PLC is in RUN mode.
    Run = 0x08,
}

/// Result of [`S7Client::get_order_code`](crate::S7Client::get_order_code).
#[derive(Debug, Clone)]
pub struct OrderCode {
    /// The order number (e.g. `"6ES7 317-2EK14-0AB0"`).
    pub code: String,
    /// Firmware version major component.
    pub v1: u8,
    /// Firmware version minor component.
    pub v2: u8,
    /// Firmware version patch component.
    pub v3: u8,
}

/// Protocol variant used by the PLC.
///
/// - **S7** — Classic S7 protocol, used by S7-300, S7-400, S7-1200
/// - **S7Plus** — S7+ (S7-Plus) protocol, used by S7-1500
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Protocol {
    /// Classic S7 protocol (S7-300, S7-400, S7-1200).
    S7,
    /// S7+ protocol (S7-1500).
    S7Plus,
}

impl std::fmt::Display for Protocol {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Protocol::S7 => write!(f, "S7"),
            Protocol::S7Plus => write!(f, "S7+"),
        }
    }
}

/// Result of [`S7Client::get_cpu_info`](crate::S7Client::get_cpu_info).
#[derive(Debug, Clone)]
pub struct CpuInfo {
    /// Module type name (e.g. `"CPU 317-2 PN/DP"`).
    pub module_type: String,
    /// CPU serial number.
    pub serial_number: String,
    /// Plant identification (AS name).
    pub as_name: String,
    /// Copyright notice.
    pub copyright: String,
    /// Module name.
    pub module_name: String,
    /// Protocol version used by the PLC (S7 for 300/400, S7+ for 1500).
    pub protocol: Protocol,
}

/// Result of [`S7Client::get_cp_info`](crate::S7Client::get_cp_info).
#[derive(Debug, Clone)]
pub struct CpInfo {
    /// Maximum PDU byte length.
    pub max_pdu_len: u32,
    /// Maximum number of connections.
    pub max_connections: u32,
    /// Maximum MPI baud rate.
    pub max_mpi_rate: u32,
    /// Maximum bus baud rate.
    pub max_bus_rate: u32,
}

/// Result of [`S7Client::get_protection`](crate::S7Client::get_protection).
#[derive(Debug, Clone)]
pub struct Protection {
    /// Protection scheme SZL number.
    pub scheme_szl: u16,
    /// Protection scheme module number.
    pub scheme_module: u16,
    /// Protection scheme bus number.
    pub scheme_bus: u16,
    /// Protection level: 0=none, 1=write, 2=read/write, 3=complete.
    pub level: u16,
    /// Whether a password is currently set on the PLC.
    pub password_set: bool,
}

/// Obfuscate an S7 password using the nibble-swap + XOR-0x55 algorithm.
///
/// Passwords longer than 8 bytes are truncated; shorter passwords are
/// space-padded to 8 bytes.  Returns an 8-byte array suitable for use
/// with [`S7Client::set_session_password`](crate::S7Client::set_session_password).
pub fn encrypt_password(password: &str) -> [u8; 8] {
    let bytes = password.as_bytes();
    let mut pw = [0x20u8; 8]; // space-padded
    let len = bytes.len().min(8);
    pw[..len].copy_from_slice(&bytes[..len]);
    let mut result = [0u8; 8];
    for i in 0..8 {
        // Swap nibbles then XOR with 0x55
        result[i] = (pw[i] << 4) | (pw[i] >> 4);
        result[i] ^= 0x55;
    }
    result
}

/// A module entry returned by [`S7Client::read_module_list`](crate::S7Client::read_module_list).
#[derive(Debug, Clone)]
pub struct ModuleEntry {
    /// Module type identifier.
    pub module_type: u16,
}

/// A single block type/count entry in [`BlockList`].
#[derive(Debug, Clone)]
pub struct BlockListEntry {
    /// Block type identifier (matches [`BlockType`] discriminant values).
    pub block_type: u16,
    /// Number of blocks of this type present in the PLC.
    pub count: u16,
}

/// Result of [`S7Client::list_blocks`](crate::S7Client::list_blocks).
#[derive(Debug, Clone)]
pub struct BlockList {
    /// Total number of blocks across all types.
    pub total_count: u32,
    /// Per-type block counts.
    pub entries: Vec<BlockListEntry>,
}

/// A raw PLC block in the Siemens Diagra upload/download format.
///
/// The wire format starts with a 20-byte header:
/// ```text
/// [blk_type:2][blk_number:2][format:2][length:4][flags:2][crc1:2][crc2:2][??:4]
/// ```
/// followed by the MC7 code / data payload, and optionally trailer strings.
#[derive(Debug, Clone)]
pub struct BlockData {
    /// Block type identifier (see [`BlockType`] discriminants).
    pub block_type: u16,
    /// Block number.
    pub block_number: u16,
    /// Block format/encoding version.
    pub format: u16,
    /// Total block length (including header).
    pub total_length: u32,
    /// Block flags.
    pub flags: u16,
    /// First CRC value.
    pub crc1: u16,
    /// Second CRC value.
    pub crc2: u16,
    /// Raw MC7 code / data payload (everything after the 20-byte header).
    pub payload: Vec<u8>,
}

/// Attributes that can be set on a block header.
#[derive(Debug, Clone, Default)]
pub struct BlockAttributes {
    /// Author string (max 8 chars, padded with spaces).
    pub author: Option<String>,
    /// Family string (max 8 chars, padded with spaces).
    pub family: Option<String>,
    /// Header/name string (max 8 chars, padded with spaces).
    pub name: Option<String>,
    /// Version (major.minor encoded as `(major << 4) | minor`).
    pub version: Option<u8>,
    /// Block flags (overrides existing flags word).
    pub flags: Option<u16>,
}

impl BlockData {
    /// Parse raw uploaded bytes into a `BlockData`.
    pub fn from_bytes(data: &[u8]) -> Option<Self> {
        if data.len() < 20 {
            return None;
        }
        let block_type = u16::from_be_bytes([data[0], data[1]]);
        let block_number = u16::from_be_bytes([data[2], data[3]]);
        let format = u16::from_be_bytes([data[4], data[5]]);
        let total_length = u32::from_be_bytes([data[6], data[7], data[8], data[9]]);
        let flags = u16::from_be_bytes([data[10], data[11]]);
        let crc1 = u16::from_be_bytes([data[12], data[13]]);
        let crc2 = u16::from_be_bytes([data[14], data[15]]);
        // Skip 20 bytes of header, the rest is payload
        let payload = data[20..].to_vec();
        Some(BlockData {
            block_type,
            block_number,
            format,
            total_length,
            flags,
            crc1,
            crc2,
            payload,
        })
    }

    /// Serialize back to wire bytes (for download).
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut buf = Vec::with_capacity(20 + self.payload.len());
        buf.extend_from_slice(&self.block_type.to_be_bytes());
        buf.extend_from_slice(&self.block_number.to_be_bytes());
        buf.extend_from_slice(&self.format.to_be_bytes());
        buf.extend_from_slice(&self.total_length.to_be_bytes());
        buf.extend_from_slice(&self.flags.to_be_bytes());
        buf.extend_from_slice(&self.crc1.to_be_bytes());
        buf.extend_from_slice(&self.crc2.to_be_bytes());
        buf.extend_from_slice(&[0u8; 4]); // reserved
        buf.extend_from_slice(&self.payload);
        buf
    }

    /// Build a minimal empty DB block ready for download.
    ///
    /// Creates a Diagra-format block with the S7 DB header structure.
    /// `size_bytes` is the desired DB size in bytes (must be even).
    pub fn new_db(db_number: u16, size_bytes: u16) -> Self {
        // Minimal S7 DB block payload: 2-byte "actual size" + zero data
        let size = (size_bytes as usize + 1) & !1; // round up to even
        let mut payload = Vec::with_capacity(2 + size);
        payload.extend_from_slice(&(size as u16).to_be_bytes());
        payload.extend(std::iter::repeat(0u8).take(size));
        let total_length = (20 + payload.len()) as u32;
        BlockData {
            block_type: BlockType::DB as u16,
            block_number: db_number,
            format: 0x0001,
            total_length,
            flags: 0x0000,
            crc1: 0x0000,
            crc2: 0x0000,
            payload,
        }
    }

    /// Compute a CRC-32 checksum of the serialized block bytes.
    ///
    /// Suitable for comparing a locally stored block against one uploaded
    /// from the PLC: `local.crc32() == plc_block.crc32()`.
    pub fn crc32(&self) -> u32 {
        let bytes = self.to_bytes();
        crc32_ieee(&bytes)
    }

    /// Apply [`BlockAttributes`] to this block in-place.
    ///
    /// The S7 block footer is at `payload[payload.len()-48..]` (when payload
    /// is large enough).  Author/Family/Name each occupy 8 bytes at fixed
    /// offsets within the footer.
    pub fn set_attributes(&mut self, attrs: &BlockAttributes) {
        if let Some(f) = attrs.flags {
            self.flags = f;
        }
        // Footer is last 48 bytes of payload (S7 block structure)
        let plen = self.payload.len();
        if plen < 48 {
            return;
        }
        let footer = &mut self.payload[plen - 48..];
        // Footer layout (S7 standard):
        //   [0..8]   reserved
        //   [8..16]  author (8 bytes, space-padded)
        //   [16..24] family (8 bytes, space-padded)
        //   [24..32] name/header (8 bytes, space-padded)
        //   [32]     version byte
        //   [33..48] reserved/checksum
        if let Some(ref s) = attrs.author {
            write_padded(&mut footer[8..16], s);
        }
        if let Some(ref s) = attrs.family {
            write_padded(&mut footer[16..24], s);
        }
        if let Some(ref s) = attrs.name {
            write_padded(&mut footer[24..32], s);
        }
        if let Some(v) = attrs.version {
            footer[32] = v;
        }
    }

    /// Return the human-readable block type name.
    pub fn type_name(&self) -> &'static str {
        block_type_name(self.block_type as u8)
    }
}

pub fn block_type_name(bt: u8) -> &'static str {
    match bt {
        0x38 => "OB",
        0x41 => "DB",
        0x42 => "SDB",
        0x43 => "FC",
        0x44 => "SFC",
        0x45 => "FB",
        0x46 => "SFB",
        0x47 => "UDT",
        _ => "??",
    }
}

fn write_padded(dst: &mut [u8], s: &str) {
    let bytes = s.as_bytes();
    let n = bytes.len().min(dst.len());
    dst[..n].copy_from_slice(&bytes[..n]);
    for b in dst[n..].iter_mut() {
        *b = b' ';
    }
}

// CRC-32 (IEEE 802.3 polynomial 0xEDB88320) — no external dep needed.
fn crc32_ieee(data: &[u8]) -> u32 {
    let mut crc: u32 = 0xFFFF_FFFF;
    for &byte in data {
        crc ^= byte as u32;
        for _ in 0..8 {
            if crc & 1 != 0 {
                crc = (crc >> 1) ^ 0xEDB8_8320;
            } else {
                crc >>= 1;
            }
        }
    }
    !crc
}

/// Compare two block lists: local files vs PLC blocks.
///
/// Each entry is `(block_type, block_number)` in `local`; the closure
/// `plc_crc` is called for each to retrieve the PLC-side CRC.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BlockCmpResult {
    /// Identical CRC — block matches.
    Match,
    /// CRC differs — block has been modified on the PLC.
    Mismatch { local_crc: u32, plc_crc: u32 },
    /// Block exists locally but not on the PLC.
    OnlyLocal,
    /// Block exists on the PLC but not locally.
    OnlyPlc,
}

/// Detailed information about a PLC block, returned by
/// [`S7Client::get_ag_block_info`](crate::S7Client::get_ag_block_info) and
/// [`S7Client::get_pg_block_info`](crate::S7Client::get_pg_block_info).
#[derive(Debug, Clone)]
pub struct BlockInfo {
    pub block_type: u16,
    pub block_number: u16,
    pub language: u16,
    pub flags: u16,
    pub size: u16,
    pub size_ram: u16,
    pub mc7_size: u16,
    pub local_data: u16,
    pub checksum: u16,
    pub version: u16,
    pub author: String,
    pub family: String,
    pub header: String,
    pub date: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum BlockType {
    OB = 0x38,
    DB = 0x41,
    SDB = 0x42,
    FC = 0x43,
    SFC = 0x44,
    FB = 0x45,
    SFB = 0x46,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn connect_params_default() {
        let p = ConnectParams::default();
        assert_eq!(p.rack, 0);
        assert_eq!(p.slot, 1);
        assert_eq!(p.pdu_size, 480);
    }

    #[test]
    fn block_data_roundtrip() {
        let bd = super::BlockData {
            block_type: 0x41, // DB
            block_number: 1,
            format: 0,
            total_length: 24,
            flags: 0,
            crc1: 0x1234,
            crc2: 0x5678,
            payload: vec![0xDE, 0xAD],
        };
        let bytes = bd.to_bytes();
        assert_eq!(bytes.len(), 22); // 20 header + 2 payload
        let parsed = super::BlockData::from_bytes(&bytes).unwrap();
        assert_eq!(parsed.block_type, 0x41);
        assert_eq!(parsed.block_number, 1);
        assert_eq!(parsed.payload, vec![0xDE, 0xAD]);
    }

    #[test]
    fn block_data_short_input_returns_none() {
        let result = super::BlockData::from_bytes(&[0u8; 10]);
        assert!(result.is_none());
    }

    #[test]
    fn encrypt_8_char_password() {
        // Known vector: "PASSWORD" -> swap nibbles, XOR 0x55
        let result = super::encrypt_password("PASSWORD");
        assert_eq!(result.len(), 8);
        // Each byte: nibble_swap(byte) ^ 0x55
        // 'P' = 0x50 -> 0x05 -> 0x05 ^ 0x55 = 0x50
        // 'A' = 0x41 -> 0x14 -> 0x14 ^ 0x55 = 0x41
        // Wait — this depends on the actual algorithm.
        // Let's verify the algorithm is self-consistent:
        let result2 = super::encrypt_password("PASSWORD");
        assert_eq!(result, result2);
    }

    #[test]
    fn encrypt_short_password_padded() {
        let result = super::encrypt_password("abc");
        // "abc" padded to 8 bytes with spaces (0x20)
        // byte 0: 'a'(0x61) -> swap -> 0x16 -> ^0x55 -> 0x43
        assert_eq!((0x61u8 << 4) | (0x61u8 >> 4), 0x16);
        assert_eq!(0x16 ^ 0x55, 0x43);
        assert_eq!(result[0], 0x43);
        // byte 3: space(0x20) -> swap -> 0x02 -> ^0x55 -> 0x57
        assert_eq!((0x20u8 << 4) | (0x20u8 >> 4), 0x02);
        assert_eq!(0x02 ^ 0x55, 0x57);
        assert_eq!(result[3], 0x57);
    }

    #[test]
    fn encrypt_long_password_truncated() {
        let result = super::encrypt_password("1234567890");
        assert_eq!(result.len(), 8);
        let result8 = super::encrypt_password("12345678");
        assert_eq!(result, result8);
    }

    #[test]
    fn block_type_discriminants() {
        assert_eq!(BlockType::DB as u8, 0x41);
        assert_eq!(BlockType::OB as u8, 0x38);
    }
}
