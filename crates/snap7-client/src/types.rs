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
    fn block_type_discriminants() {
        assert_eq!(BlockType::DB as u8, 0x41);
        assert_eq!(BlockType::OB as u8, 0x38);
    }
}
