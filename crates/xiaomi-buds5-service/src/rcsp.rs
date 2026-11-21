//! RCSP framing, command authentication, and device-state decoding.
//!
//! A port of `src/xiaomi_buds5_panel/rcsp.py`. The Xiaomi Buds 5 exposes its
//! control endpoint as Bluetooth Classic RFCOMM (channel 29 on this machine).
//! The audio stream stays a normal BlueZ A2DP stream; this module only owns the
//! small control channel.

use std::io::Read;
use std::os::unix::io::RawFd;
use std::time::{Duration, Instant};

use crate::auth::{transform, ADDR6_PLACEHOLDER};

pub const MAGIC: [u8; 3] = [0xFE, 0xDC, 0xBA];
pub const TRAILER: u8 = 0xEF;
pub const APP_MIUI: u8 = 1;
pub const APP_EARPHONE: u8 = 4;
pub const RFCOMM_CHANNELS: [u8; 9] = [29, 11, 27, 6, 5, 4, 3, 2, 1];

// Linux constants (libc does not export sockaddr_rc / BTPROTO_RFCOMM).
const AF_BLUETOOTH: i32 = 31;
const BTPROTO_RFCOMM: i32 = 3;
const EAGAIN: i32 = 11;
const EBUSY: i32 = 16;
const EHOSTDOWN: i32 = 112;
const EHOSTUNREACH: i32 = 113;

/// A recoverable protocol or Bluetooth error.
#[derive(Debug)]
pub struct BudsError {
    pub msg: String,
    pub errno: Option<i32>,
}

impl BudsError {
    pub fn new(msg: impl Into<String>) -> Self {
        BudsError { msg: msg.into(), errno: None }
    }
    pub fn os(context: &str, errno: i32) -> Self {
        BudsError {
            msg: format!("{context}: {}", std::io::Error::from_raw_os_error(errno)),
            errno: Some(errno),
        }
    }
}

impl std::fmt::Display for BudsError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.msg)
    }
}
impl std::error::Error for BudsError {}

pub type Result<T> = std::result::Result<T, BudsError>;

fn status_name(status: u8) -> String {
    match status {
        0 => "ok".into(),
        1 => "失败".into(),
        2 => "未知命令".into(),
        3 => "忙".into(),
        4 => "资源不足".into(),
        5 => "校验错误".into(),
        7 => "参数错误".into(),
        9 => "不支持".into(),
        other => format!("状态 {other}"),
    }
}

#[derive(Clone, Debug)]
pub struct Cell {
    pub percent: Option<u8>,
    pub charging: bool,
}

#[derive(Clone, Debug)]
pub struct Frame {
    pub flags: u8,
    pub opcode: u8,
    pub payload: Vec<u8>,
}

impl Frame {
    pub fn is_command(&self) -> bool {
        self.flags & 0x80 != 0
    }
    pub fn status(&self) -> u8 {
        *self.payload.first().unwrap_or(&0xFF)
    }
    pub fn sequence(&self) -> u8 {
        if self.is_command() {
            *self.payload.first().unwrap_or(&0xFF)
        } else if self.payload.len() > 1 {
            self.payload[1]
        } else {
            0xFF
        }
    }
    pub fn data(&self) -> &[u8] {
        let skip = if self.is_command() {
            if self.opcode == 0x01 { 2 } else { 1 }
        } else if self.opcode == 0x01 {
            1
        } else {
            2
        };
        if self.payload.len() >= skip {
            &self.payload[skip..]
        } else {
            &[]
        }
    }
    pub fn encode(flags: u8, opcode: u8, payload: &[u8]) -> Vec<u8> {
        let mut out = Vec::with_capacity(7 + payload.len());
        out.extend_from_slice(&MAGIC);
        out.push(flags);
        out.push(opcode);
        out.extend_from_slice(&(payload.len() as u16).to_be_bytes());
        out.extend_from_slice(payload);
        out.push(TRAILER);
        out
    }
}

fn find_magic(buffer: &[u8]) -> Option<usize> {
    buffer.windows(3).position(|w| w == MAGIC)
}

/// Pull every complete, trailer-terminated frame out of `buffer` (mutating it).
pub fn extract_frames(buffer: &mut Vec<u8>) -> Vec<Frame> {
    let mut frames = Vec::new();
    loop {
        let start = match find_magic(buffer) {
            None => {
                // Keep the last 2 bytes: a magic may straddle the next chunk.
                if buffer.len() > 2 {
                    let keep = buffer.len() - 2;
                    buffer.drain(0..keep);
                }
                return frames;
            }
            Some(s) => s,
        };
        if start > 0 {
            buffer.drain(0..start);
        }
        if buffer.len() < 7 {
            return frames;
        }
        let size = u16::from_be_bytes([buffer[5], buffer[6]]) as usize;
        let end = 7 + size;
        if buffer.len() <= end {
            return frames;
        }
        if buffer[end] != TRAILER {
            buffer.remove(0);
            continue;
        }
        frames.push(Frame {
            flags: buffer[3],
            opcode: buffer[4],
            payload: buffer[7..end].to_vec(),
        });
        buffer.drain(0..end + 1);
    }
}

pub fn parse_tlvs_u16(data: &[u8]) -> Vec<(u16, Vec<u8>)> {
    let mut out = Vec::new();
    let mut pos = 0;
    while pos + 3 <= data.len() {
        let length = data[pos] as usize;
        let end = pos + 1 + length;
        if length < 2 || end > data.len() {
            break;
        }
        let id = u16::from_be_bytes([data[pos + 1], data[pos + 2]]);
        out.push((id, data[pos + 3..end].to_vec()));
        pos = end;
    }
    out
}

pub fn parse_tlvs_u8(data: &[u8]) -> Vec<(u8, Vec<u8>)> {
    let mut out = Vec::new();
    let mut pos = 0;
    while pos + 2 <= data.len() {
        let length = data[pos] as usize;
        let end = pos + 1 + length;
        if length < 1 || end > data.len() {
            break;
        }
        out.push((data[pos + 1], data[pos + 2..end].to_vec()));
        pos = end;
    }
    out
}

fn tlv_get_u8(list: &[(u8, Vec<u8>)], id: u8) -> Option<&[u8]> {
    list.iter().find(|(k, _)| *k == id).map(|(_, v)| v.as_slice())
}
fn tlv_get_u16(list: &[(u16, Vec<u8>)], id: u16) -> Option<&[u8]> {
    list.iter().find(|(k, _)| *k == id).map(|(_, v)| v.as_slice())
}

pub fn tlv_u16(config_id: u16, value: &[u8]) -> Vec<u8> {
    let mut out = vec![(2 + value.len()) as u8];
    out.extend_from_slice(&config_id.to_be_bytes());
    out.extend_from_slice(value);
    out
}

pub fn decode_battery(value: &[u8]) -> Vec<Cell> {
    value
        .iter()
        .map(|&raw| {
            if raw == 0xFF {
                Cell { percent: None, charging: false }
            } else {
                Cell { percent: Some(raw & 0x7F), charging: raw & 0x80 != 0 }
            }
        })
        .collect()
}

pub fn parse_battery_notification(payload: &[u8]) -> Option<Vec<Cell>> {
    if payload.len() >= 5 && payload[0] == 4 && payload[1] == 0 {
        return Some(decode_battery(&payload[2..5]));
    }
    if payload.len() >= 6 && payload[1] == 4 && payload[2] == 0 {
        return Some(decode_battery(&payload[3..6]));
    }
    None
}

// --- RFCOMM socket -------------------------------------------------------

#[repr(C)]
struct SockAddrRc {
    rc_family: u16,
    rc_bdaddr: [u8; 6],
    rc_channel: u8,
}

struct RfcommSocket {
    fd: RawFd,
}

fn last_errno() -> i32 {
    std::io::Error::last_os_error().raw_os_error().unwrap_or(0)
}

fn set_sock_timeout(fd: RawFd, opt: i32, d: Duration) {
    let tv = libc::timeval {
        tv_sec: d.as_secs() as libc::time_t,
        tv_usec: d.subsec_micros() as libc::suseconds_t,
    };
    unsafe {
        libc::setsockopt(
            fd,
            libc::SOL_SOCKET,
            opt,
            &tv as *const _ as *const libc::c_void,
            std::mem::size_of::<libc::timeval>() as libc::socklen_t,
        );
    }
}

fn parse_addr(address: &str) -> Result<[u8; 6]> {
    let parts: Vec<&str> = address.split(':').collect();
    if parts.len() != 6 {
        return Err(BudsError::new(format!("无效的蓝牙地址: {address}")));
    }
    let mut raw = [0u8; 6];
    for (i, p) in parts.iter().enumerate() {
        raw[i] = u8::from_str_radix(p, 16)
            .map_err(|_| BudsError::new(format!("无效的蓝牙地址: {address}")))?;
    }
    // the kernel wants bdaddr_t in little-endian display order
    let mut bd = [0u8; 6];
    for i in 0..6 {
        bd[i] = raw[5 - i];
    }
    Ok(bd)
}

impl RfcommSocket {
    fn connect(address: &str, channel: u8, timeout: Duration) -> Result<Self> {
        let bd = parse_addr(address)?;
        let fd = unsafe { libc::socket(AF_BLUETOOTH, libc::SOCK_STREAM, BTPROTO_RFCOMM) };
        if fd < 0 {
            return Err(BudsError::os("socket", last_errno()));
        }
        set_sock_timeout(fd, libc::SO_RCVTIMEO, timeout);
        set_sock_timeout(fd, libc::SO_SNDTIMEO, timeout);
        let sa = SockAddrRc {
            rc_family: AF_BLUETOOTH as u16,
            rc_bdaddr: bd,
            rc_channel: channel,
        };
        let rc = unsafe {
            libc::connect(
                fd,
                &sa as *const _ as *const libc::sockaddr,
                std::mem::size_of::<SockAddrRc>() as libc::socklen_t,
            )
        };
        if rc < 0 {
            let e = last_errno();
            unsafe { libc::close(fd) };
            return Err(BudsError::os(&format!("connect ch{channel}"), e));
        }
        Ok(RfcommSocket { fd })
    }

    fn set_read_timeout(&self, d: Duration) {
        set_sock_timeout(self.fd, libc::SO_RCVTIMEO, d);
    }

    fn recv(&self, buf: &mut [u8]) -> Result<Option<usize>> {
        let n = unsafe { libc::recv(self.fd, buf.as_mut_ptr() as *mut libc::c_void, buf.len(), 0) };
        if n < 0 {
            let e = last_errno();
            if e == EAGAIN || e == libc::EWOULDBLOCK {
                return Ok(None);
            }
            return Err(BudsError::os("recv", e));
        }
        if n == 0 {
            return Err(BudsError::new("耳机断开了控制连接"));
        }
        Ok(Some(n as usize))
    }

    fn send_all(&self, mut data: &[u8]) -> Result<()> {
        while !data.is_empty() {
            let n = unsafe {
                libc::send(self.fd, data.as_ptr() as *const libc::c_void, data.len(), libc::MSG_NOSIGNAL)
            };
            if n < 0 {
                return Err(BudsError::os("send", last_errno()));
            }
            data = &data[n as usize..];
        }
        Ok(())
    }
}

impl Drop for RfcommSocket {
    fn drop(&mut self) {
        unsafe { libc::close(self.fd) };
    }
}

// --- session -------------------------------------------------------------

fn urandom16() -> [u8; 16] {
    let mut buf = [0u8; 16];
    if let Ok(mut f) = std::fs::File::open("/dev/urandom") {
        let _ = f.read_exact(&mut buf);
    }
    buf
}

pub struct Info {
    pub name: String,
    pub battery: Vec<Cell>,
    #[allow(dead_code)]
    pub pid: Option<u16>,
}

pub struct BudsSession {
    pub address: String,
    pub channel: Option<u8>,
    sock: Option<RfcommSocket>,
    sequence: u8,
    buffer: Vec<u8>,
    pending: Vec<Frame>,
    verbose: bool,
}

impl BudsSession {
    pub fn new(address: &str, verbose: bool) -> Self {
        BudsSession {
            address: address.to_string(),
            channel: None,
            sock: None,
            sequence: 0,
            buffer: Vec::new(),
            pending: Vec::new(),
            verbose,
        }
    }

    fn log(&self, text: &str) {
        if self.verbose {
            eprintln!("[xiaomi-buds5] {text}");
        }
    }

    pub fn connect(&mut self) -> Result<()> {
        self.close();
        self.pending.clear();
        let mut last: Option<BudsError> = None;
        for &channel in RFCOMM_CHANNELS.iter() {
            let attempts = if channel == 29 { 3 } else { 1 };
            for attempt in 0..attempts {
                if attempt > 0 {
                    std::thread::sleep(Duration::from_millis(800));
                }
                match RfcommSocket::connect(&self.address, channel, Duration::from_secs(2)) {
                    Ok(sock) => {
                        self.sock = Some(sock);
                        self.channel = Some(channel);
                        self.buffer.clear();
                        self.pending.clear();
                        self.log(&format!("RFCOMM channel {channel}"));
                        return Ok(());
                    }
                    Err(exc) => {
                        let e = exc.errno;
                        last = Some(exc);
                        if e == Some(EBUSY) && attempt + 1 < attempts {
                            continue;
                        }
                        if e == Some(EHOSTDOWN) || e == Some(EHOSTUNREACH) {
                            break;
                        }
                        if e == Some(EBUSY) {
                            return Err(BudsError::new("蓝牙控制通道不可用"));
                        }
                    }
                }
            }
        }
        Err(BudsError::new(match last {
            Some(e) => format!("无法连接耳机控制通道: {e}"),
            None => "无法连接耳机控制通道: 未知错误".into(),
        }))
    }

    pub fn close(&mut self) {
        self.sock = None;
    }

    fn next_sequence(&mut self) -> u8 {
        self.sequence = self.sequence.wrapping_add(1);
        self.sequence
    }

    fn send_frame(&self, flags: u8, opcode: u8, payload: &[u8]) -> Result<()> {
        let sock = self
            .sock
            .as_ref()
            .ok_or_else(|| BudsError::new("蓝牙控制通道未连接"))?;
        sock.send_all(&Frame::encode(flags, opcode, payload))
    }

    fn send_command(&mut self, opcode: u8, params: &[u8], app: u8) -> Result<u8> {
        let seq = self.next_sequence();
        let mut payload = vec![seq];
        payload.extend_from_slice(params);
        self.send_frame(0x80 | 0x40 | app, opcode, &payload)?;
        Ok(seq)
    }

    pub fn recv_frame(&mut self, timeout: Duration) -> Result<Option<Frame>> {
        if !self.pending.is_empty() {
            return Ok(Some(self.pending.remove(0)));
        }
        let deadline = Instant::now() + timeout;
        loop {
            let frames = extract_frames(&mut self.buffer);
            if !frames.is_empty() {
                let mut iter = frames.into_iter();
                let first = iter.next().unwrap();
                self.pending.extend(iter);
                return Ok(Some(first));
            }
            let now = Instant::now();
            if now >= deadline {
                return Ok(None);
            }
            let remaining = deadline - now;
            let sock = self
                .sock
                .as_ref()
                .ok_or_else(|| BudsError::new("蓝牙控制通道未连接"))?;
            sock.set_read_timeout(remaining.max(Duration::from_millis(50)));
            let mut chunk = [0u8; 4096];
            match sock.recv(&mut chunk)? {
                None => return Ok(None), // timeout
                Some(n) => self.buffer.extend_from_slice(&chunk[..n]),
            }
        }
    }

    fn answer_device_challenge(&mut self, frame: &Frame) -> Result<bool> {
        if !frame.is_command() || frame.opcode != 0x50 {
            return Ok(false);
        }
        let body = frame.data();
        if body.len() < 17 || body[0] != 1 {
            return Ok(false);
        }
        let answer = transform(&ADDR6_PLACEHOLDER, &body[1..17].try_into().unwrap());
        let mut payload = vec![0u8, frame.sequence(), 1u8];
        payload.extend_from_slice(&answer);
        self.send_frame(APP_MIUI, 0x50, &payload)?;
        self.log("answered device challenge");
        Ok(true)
    }

    fn wait_response(&mut self, seq: u8, opcode: u8, timeout: Duration) -> Result<Frame> {
        let deadline = Instant::now() + timeout;
        loop {
            let now = Instant::now();
            if now >= deadline {
                return Err(BudsError::new("耳机没有响应"));
            }
            let frame = match self.recv_frame(deadline - now)? {
                Some(f) => f,
                None => return Err(BudsError::new("耳机没有响应")),
            };
            if self.answer_device_challenge(&frame)? {
                continue;
            }
            if frame.is_command() {
                continue;
            }
            if frame.sequence() == seq && frame.opcode == opcode {
                if frame.status() != 0 {
                    return Err(BudsError::new(status_name(frame.status())));
                }
                return Ok(frame);
            }
        }
    }

    fn request(&mut self, opcode: u8, params: &[u8], app: u8) -> Result<Frame> {
        let seq = self.send_command(opcode, params, app)?;
        self.wait_response(seq, opcode, Duration::from_secs(3))
    }

    pub fn authenticate(&mut self) -> Result<()> {
        let challenge = urandom16();
        let expected = transform(&ADDR6_PLACEHOLDER, &challenge);
        let mut params = vec![1u8];
        params.extend_from_slice(&challenge);
        let seq = self.send_command(0x50, &params, APP_MIUI)?;
        let response = self.wait_response(seq, 0x50, Duration::from_secs(5))?;
        let data = response.data();
        if data.len() < 17 || data[1..17] != expected {
            return Err(BudsError::new("耳机认证响应不匹配"));
        }
        self.request(0x51, &[1, 0], APP_MIUI)?;
        Ok(())
    }

    pub fn target_info(&mut self) -> Result<Info> {
        let response = self.request(0x02, &[0xFF, 0xFF, 0xFF, 0xFF], APP_EARPHONE)?;
        let attrs = parse_tlvs_u8(response.data());
        let name = tlv_get_u8(&attrs, 0)
            .map(|b| {
                let end = b.iter().position(|&c| c == 0).unwrap_or(b.len());
                String::from_utf8_lossy(&b[..end]).trim().to_string()
            })
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| "Xiaomi Buds 5".to_string());
        let battery = decode_battery(tlv_get_u8(&attrs, 7).unwrap_or(&[]));
        let pid = tlv_get_u8(&attrs, 3)
            .filter(|b| b.len() >= 4)
            .map(|b| u16::from_be_bytes([b[2], b[3]]));
        Ok(Info { name, battery, pid })
    }

    pub fn battery(&mut self) -> Result<Vec<Cell>> {
        Ok(self.target_info()?.battery)
    }

    fn get_config(&mut self, ids: &[u16]) -> Result<Vec<(u16, Vec<u8>)>> {
        let mut params = Vec::new();
        for &id in ids {
            params.extend_from_slice(&id.to_be_bytes());
        }
        let response = self.request(0xF3, &params, APP_EARPHONE)?;
        Ok(parse_tlvs_u16(response.data()))
    }

    fn set_config(&mut self, config_id: u16, value: &[u8]) -> Result<()> {
        self.request(0xF2, &tlv_u16(config_id, value), APP_EARPHONE)?;
        Ok(())
    }

    pub fn noise_state(&mut self) -> Result<Option<(u8, u8)>> {
        let cfg = self.get_config(&[11])?;
        match tlv_get_u16(&cfg, 11) {
            Some(v) if v.len() >= 2 => Ok(Some((v[0], v[1]))),
            _ => Ok(None),
        }
    }

    pub fn set_noise(&mut self, mode: u8) -> Result<()> {
        if mode > 2 {
            return Err(BudsError::new("noise mode must be 0, 1, or 2"));
        }
        // The DSP is driven by SET_TARGET_INFO attr 4, not by config id 11.
        self.request(0x08, &[2, 4, mode], APP_EARPHONE)?;
        Ok(())
    }

    pub fn set_noise_level(&mut self, mode: u8, level: u8) -> Result<()> {
        if !(mode == 1 || mode == 2) || level > 2 {
            return Err(BudsError::new("invalid noise level"));
        }
        self.set_config(11, &[mode, level])
    }

    pub fn poll_battery(&mut self, timeout: Duration) -> Result<Option<Vec<Cell>>> {
        let deadline = Instant::now() + timeout;
        loop {
            let now = Instant::now();
            if now >= deadline {
                return Ok(None);
            }
            let frame = match self.recv_frame(deadline - now)? {
                Some(f) => f,
                None => return Ok(None),
            };
            if self.answer_device_challenge(&frame)? {
                continue;
            }
            if frame.opcode == 0x0E && frame.is_command() {
                if let Some(parsed) = parse_battery_notification(frame.data()) {
                    return Ok(Some(parsed));
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Ported from the deleted tests/test_rcsp.py so the protocol stays covered.
    #[test]
    fn frame_round_trip_and_tlv() {
        let raw = Frame::encode(0xC4, 0xF3, b"\x01\x00\x0b");
        let mut buf = raw.clone();
        let frames = extract_frames(&mut buf);
        assert_eq!(frames.len(), 1);
        assert_eq!(frames[0].opcode, 0xF3);
        assert_eq!(parse_tlvs_u16(b"\x04\x00\x0b\x01\x02"), vec![(11u16, vec![0x01, 0x02])]);
    }

    #[test]
    fn battery_records() {
        let cells = decode_battery(&[0xE1, 0x63, 0xFF]);
        assert_eq!(cells.len(), 3);
        assert_eq!((cells[0].percent, cells[0].charging), (Some(97), true));
        assert_eq!((cells[1].percent, cells[1].charging), (Some(99), false));
        assert_eq!((cells[2].percent, cells[2].charging), (None, false));

        let notif = parse_battery_notification(&[0x04, 0x00, 0x61, 0x5f, 0xff, 0x02, 0x04, 0x00]).unwrap();
        assert_eq!(notif.len(), 3);
        assert_eq!((notif[0].percent, notif[0].charging), (Some(97), false));
        assert_eq!((notif[1].percent, notif[1].charging), (Some(95), false));
        assert_eq!((notif[2].percent, notif[2].charging), (None, false));
    }
}
