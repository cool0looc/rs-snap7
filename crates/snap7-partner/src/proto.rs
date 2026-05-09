/// Group byte for BSend/BRecv (0x46 = send, 0x86 = send-ack).
pub const GR_BSEND: u8 = 0x46;
pub const GR_BSEND_ACK: u8 = 0x86;

/// S7 request header size (fixed, no error class/code for UserData).
pub const S7_HDR_LEN: usize = 10;
/// TBSendParams size: 12 bytes.
pub const BSEND_PARAMS_LEN: usize = 12;
/// TBsendRequestData size: 12 bytes (FF + TRSize + Len(2) + DHead(4) + R_ID(4)).
pub const BSEND_DATA_HDR_LEN: usize = 12;

// ---------------------------------------------------------------------------
// BSend param block (TBSendParams, 12 bytes)
// ---------------------------------------------------------------------------

pub struct BSendParams {
    pub tg: u8,
    pub sub_fun: u8,
    pub seq: u8,
    pub id_seq: u8,
    pub eos: u8,
    pub err: u16,
}

impl BSendParams {
    pub fn encode(&self, buf: &mut Vec<u8>) {
        buf.push(0x00); buf.push(0x01); buf.push(0x12); // Head
        buf.push(0x08);                                  // Plen
        buf.push(0x12);                                  // Uk
        buf.push(self.tg);
        buf.push(self.sub_fun);
        buf.push(self.seq);
        buf.push(self.id_seq);
        buf.push(self.eos);
        buf.push((self.err >> 8) as u8);
        buf.push(self.err as u8);
    }

    pub fn decode(data: &[u8]) -> Option<Self> {
        if data.len() < BSEND_PARAMS_LEN {
            return None;
        }
        Some(Self {
            tg:      data[5],
            sub_fun: data[6],
            seq:     data[7],
            id_seq:  data[8],
            eos:     data[9],
            err:     u16::from_be_bytes([data[10], data[11]]),
        })
    }
}

// ---------------------------------------------------------------------------
// BSend data header (TBsendRequestData, 12 bytes)
// ---------------------------------------------------------------------------

pub struct BSendDataHdr {
    pub len: u16,  // Slice + 8 (+ 2 if first)
    pub r_id: u32,
}

impl BSendDataHdr {
    pub fn encode(&self, buf: &mut Vec<u8>) {
        buf.push(0xFF);    // FF
        buf.push(0x09);    // TRSize = TS_ResOctet
        buf.push((self.len >> 8) as u8);
        buf.push(self.len as u8);
        buf.extend_from_slice(&[0x12, 0x06, 0x13, 0x00]); // DHead
        buf.push((self.r_id >> 24) as u8);
        buf.push((self.r_id >> 16) as u8);
        buf.push((self.r_id >> 8) as u8);
        buf.push(self.r_id as u8);
    }

    pub fn decode(data: &[u8]) -> Option<Self> {
        if data.len() < BSEND_DATA_HDR_LEN {
            return None;
        }
        let len = u16::from_be_bytes([data[2], data[3]]);
        let r_id = u32::from_be_bytes([data[8], data[9], data[10], data[11]]);
        Some(Self { len, r_id })
    }
}
