//! Logitech Litra control over HID++ 2.0 (USB and BLE HID).
pub use hidapi::HidApi;
use hidapi::HidDevice;
use std::time::{Duration, Instant};

const VID: u16 = 0x046d;
const USAGE_PAGE: u16 = 0xff43;
/// (pid, name, has_back_light)
const MODELS: &[(u16, &str, bool)] = &[
    (0xc900, "Litra Glow", false),
    (0xc901, "Litra Beam", false),
    (0xb901, "Litra Beam", false),
    (0xc903, "Litra Beam LX", true),
    (0xb903, "Litra Beam LX (Bluetooth)", true),
];

// HID++ feature ids, resolved to per-firmware indexes through IRoot.
const F_ILLUMINATION: u16 = 0x1990;
const F_BACK: u16 = 0x8040;
/// Back colour feature: id not yet identified, litra-rs documents index 0x0c on Beam LX.
const IDX_BACK_COLOR: u8 = 0x0c;

#[derive(Debug)]
pub enum Error {
    NotFound,
    Hid(hidapi::HidError),
    Rejected(u8),
    Timeout,
    Unsupported(u16),
    Range(&'static str),
}
impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Error::NotFound => write!(f, "no Litra found"),
            Error::Hid(e) => write!(f, "hid: {e}"),
            Error::Rejected(c) => write!(f, "device rejected command (code {c:#04x})"),
            Error::Timeout => write!(f, "no reply"),
            Error::Unsupported(id) => write!(f, "feature {id:#06x} not supported by this device"),
            Error::Range(what) => write!(f, "value out of range: {what}"),
        }
    }
}
impl std::error::Error for Error {}
impl From<hidapi::HidError> for Error {
    fn from(e: hidapi::HidError) -> Self { Error::Hid(e) }
}
pub type Result<T> = std::result::Result<T, Error>;

#[derive(Debug, Clone)]
pub struct Info {
    pub name: String,
    pub pid: u16,
    pub path: String,
    pub serial: String,
    pub has_back: bool,
}

pub fn list(api: &HidApi) -> Vec<Info> {
    let mut out: Vec<Info> = vec![];
    for d in api.device_list().filter(|d| d.vendor_id() == VID) {
        let Some(&(pid, name, has_back)) = MODELS.iter().find(|m| m.0 == d.product_id()) else { continue };
        // One physical light exposes several HID collections; keep the vendor one.
        let vendor = d.usage_page() == USAGE_PAGE;
        let serial = d.serial_number().unwrap_or("").to_string();
        if let Some(e) = out.iter_mut().find(|e| e.pid == pid && e.serial == serial) {
            if vendor { e.path = d.path().to_string_lossy().into(); }
            continue;
        }
        out.push(Info { name: name.into(), pid, path: d.path().to_string_lossy().into(), serial, has_back });
    }
    // Prefer the Bluetooth entry; the OS may report no serial, so never rely on it.
    out.sort_by_key(|i| if i.pid == 0xb903 { 0 } else { 1 });
    out
}

pub struct Light {
    dev: HidDevice,
    pub info: Info,
    front: u8,
    back: Option<u8>,
}

#[derive(Debug, Clone, Copy)]
pub struct Range { pub min: u16, pub max: u16 }
pub const FRONT_BRIGHTNESS: Range = Range { min: 30, max: 400 };
pub const FRONT_TEMP: Range = Range { min: 2700, max: 6500 };

impl Light {
    pub fn open(api: &HidApi, info: &Info) -> Result<Light> {
        let path = std::ffi::CString::new(info.path.clone()).unwrap();
        let dev = api.open_path(&path)?;
        let mut l = Light { dev, info: info.clone(), front: 0, back: None };
        l.front = l.feature(F_ILLUMINATION)?;
        l.back = if info.has_back { l.feature(F_BACK).ok() } else { None };
        Ok(l)
    }

    pub fn open_first(api: &HidApi) -> Result<Light> {
        let info = list(api).into_iter().next().ok_or(Error::NotFound)?;
        Light::open(api, &info)
    }

    /// Send a HID++ long report and wait for the matching reply; returns payload after the 4-byte header.
    fn call(&self, idx: u8, fn_sw: u8, args: &[u8]) -> Result<Vec<u8>> {
        let mut p = [0u8; 20];
        p[0] = 0x11; p[1] = 0xff; p[2] = idx; p[3] = fn_sw;
        p[4..4 + args.len()].copy_from_slice(args);
        self.dev.write(&p)?;
        let end = Instant::now() + Duration::from_millis(1500);
        let mut buf = [0u8; 32];
        while Instant::now() < end {
            let n = self.dev.read_timeout(&mut buf, 200)?;
            if n < 5 || buf[0] != 0x11 { continue; }
            if buf[2] == 0xff && buf[3] == idx && buf[4] == fn_sw { return Err(Error::Rejected(buf[5])); }
            if buf[2] == idx && buf[3] == fn_sw { return Ok(buf[4..n].to_vec()); }
        }
        Err(Error::Timeout)
    }

    fn feature(&self, id: u16) -> Result<u8> {
        let r = self.call(0x00, 0x0a, &id.to_be_bytes())?;
        match r.first() { Some(&i) if i != 0 => Ok(i), _ => Err(Error::Unsupported(id)) }
    }

    // ---- front light ----
    pub fn is_on(&self) -> Result<bool> { Ok(self.call(self.front, 0x01, &[])?[0] == 1) }
    pub fn set_on(&self, on: bool) -> Result<()> { self.call(self.front, 0x1c, &[on as u8]).map(drop) }
    pub fn brightness(&self) -> Result<u16> { let r = self.call(self.front, 0x31, &[])?; Ok(u16::from_be_bytes([r[0], r[1]])) }
    pub fn set_brightness(&self, lm: u16) -> Result<()> {
        if !(FRONT_BRIGHTNESS.min..=FRONT_BRIGHTNESS.max).contains(&lm) { return Err(Error::Range("brightness 30..400 lm")); }
        self.call(self.front, 0x4c, &lm.to_be_bytes()).map(drop)
    }
    pub fn temperature(&self) -> Result<u16> { let r = self.call(self.front, 0x81, &[])?; Ok(u16::from_be_bytes([r[0], r[1]])) }
    pub fn set_temperature(&self, k: u16) -> Result<()> {
        if !(FRONT_TEMP.min..=FRONT_TEMP.max).contains(&k) { return Err(Error::Range("temperature 2700..6500 K")); }
        // The firmware answers anything between with error 0x02.
        if k % 100 != 0 { return Err(Error::Range("temperature in steps of 100 K")); }
        self.call(self.front, 0x9c, &k.to_be_bytes()).map(drop)
    }

    // ---- back RGB light (Beam LX) ----
    fn back_idx(&self) -> Result<u8> { self.back.ok_or(Error::Unsupported(F_BACK)) }
    pub fn back_is_on(&self) -> Result<bool> { Ok(self.call(self.back_idx()?, 0x3b, &[])?[0] == 1) }
    pub fn set_back_on(&self, on: bool) -> Result<()> { self.call(self.back_idx()?, 0x4b, &[on as u8]).map(drop) }
    pub fn back_brightness(&self) -> Result<u8> { Ok(self.call(self.back_idx()?, 0x1b, &[])?[1]) }
    pub fn set_back_brightness(&self, pct: u8) -> Result<()> {
        if !(1..=100).contains(&pct) { return Err(Error::Range("back brightness 1..100 %")); }
        self.call(self.back_idx()?, 0x2b, &[0, pct]).map(drop)
    }
    /// Set any of the 7 back zones (1..=7, left to right) as one frame. A report carries up to
    /// four zones and nothing shows until the commit, so all seven change together in three
    /// round trips.
    pub fn set_back_zones(&self, zones: &[(u8, [u8; 3])]) -> Result<()> {
        if zones.iter().any(|(z, _)| !(1..=7).contains(z)) { return Err(Error::Range("zone 1..7")); }
        self.back_idx()?;
        if zones.is_empty() { return Ok(()); }
        for chunk in zones.chunks(4) {
            // an unused slot is marked by zone 0xff
            let mut args = [0xff, 0, 0, 0, 0xff, 0, 0, 0, 0xff, 0, 0, 0, 0xff, 0, 0, 0];
            for (slot, (zone, rgb)) in chunk.iter().enumerate() {
                let c = rgb.map(|v| v.max(1));
                args[slot * 4..slot * 4 + 4].copy_from_slice(&[*zone, c[0], c[1], c[2]]);
            }
            self.call(IDX_BACK_COLOR, 0x1b, &args)?;
        }
        self.call(IDX_BACK_COLOR, 0x7b, &[0, 0, 1, 0, 0]).map(drop)
    }
    pub fn set_back_zone(&self, zone: u8, rgb: [u8; 3]) -> Result<()> { self.set_back_zones(&[(zone, rgb)]) }
    pub fn set_back_color(&self, rgb: [u8; 3]) -> Result<()> {
        let all: Vec<(u8, [u8; 3])> = (1..=7).map(|z| (z, rgb)).collect();
        self.set_back_zones(&all)
    }
}
