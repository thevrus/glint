use hidapi::HidApi;
use glint_core::*;

fn hex(s: &str) -> Option<[u8; 3]> {
    let s = s.trim_start_matches('#');
    let v = u32::from_str_radix(s, 16).ok()?;
    (s.len() == 6).then_some([(v >> 16) as u8, (v >> 8) as u8, v as u8])
}

fn main() -> Result<()> {
    let api = HidApi::new()?;
    let a: Vec<String> = std::env::args().skip(1).collect();
    if a.first().map(String::as_str) == Some("devices") {
        for i in list(&api) { println!("{i:?}"); }
        return Ok(());
    }
    let l = Light::open_first(&api)?;
    let n = |i: usize| a.get(i).and_then(|s| s.parse::<u16>().ok());
    match a.first().map(String::as_str) {
        Some("on") => l.set_on(true)?,
        Some("off") => l.set_on(false)?,
        Some("brightness") => l.set_brightness(n(1).ok_or(Error::Range("number"))?)?,
        Some("temp") => l.set_temperature(n(1).ok_or(Error::Range("number"))?)?,
        Some("back-on") => l.set_back_on(true)?,
        Some("back-off") => l.set_back_on(false)?,
        Some("back-brightness") => l.set_back_brightness(n(1).ok_or(Error::Range("number"))? as u8)?,
        Some("back-color") => {
            let rgb = a.get(1).and_then(|s| hex(s)).ok_or(Error::Range("hex colour"))?;
            match n(2) { Some(z) => l.set_back_zone(z as u8, rgb)?, None => l.set_back_color(rgb)? }
        }
        _ => {}
    }
    println!("{}: front {} {} lm {} K", l.info.name, if l.is_on()? { "on" } else { "off" }, l.brightness()?, l.temperature()?);
    if l.info.has_back { println!("back: {} {} %", if l.back_is_on()? { "on" } else { "off" }, l.back_brightness()?); }
    Ok(())
}
