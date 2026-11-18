//! VIA (QMK raw HID) キーボード — Wobkey Rainy 75 Pro (有線 320f:5088) で実機確認。
//!
//! - インターフェース: usage page 0xFF60 / usage 0x61、32B の output/input report
//!   (report ID なし。hidapi では先頭に 0 を付けて 33B で書く)。
//! - 応答は先頭バイトにコマンドを echo する。
//! - 使うコマンド (VIA プロトコル):
//!   0x01 get_protocol_version → [1, hi, lo] (実機 0x000B)
//!   0x11 dynamic_keymap_get_layer_count → [0x11, n] (実機 4)
//!   0x12 dynamic_keymap_get_buffer [off_hi, off_lo, len≤28] → データは r[4..]
//!   0x05 dynamic_keymap_set_keycode [layer, row, col, hi, lo] (EEPROM に即保存)
//! - キーマップ buffer は `layer × rows × cols × 2B` (big endian)。Rainy 75 Pro は 6×16。
//! - キーコードは QMK の新体系 (protocol ≥ 11 / 実機 MO(1)=0x5221 で確認)。
//!
//! 2.4G ドングル経由では VIA は応答しない (別の独自プロトコル) — 有線のみ。

use devhub_core::backend::*;
use devhub_core::model::*;
use std::collections::BTreeMap;
use std::sync::Mutex;

/// VIA の 1 フレーム (report ID を除く)。
const FRAME: usize = 32;
/// get_buffer 1 回で読めるバイト数 (32 - ヘッダ 4)。
const CHUNK: u16 = 28;

/// 機種ごとのマトリクスと物理配置。
pub struct ViaModel {
    pub rows: u8,
    pub cols: u8,
    pub layout: fn() -> Vec<LayoutKey>,
}

/// 今は Rainy 75 Pro だけ。機種を足すときは model 文字列で分岐する。
fn model_for(_model: &str) -> &'static ViaModel {
    &RAINY75
}

const RAINY75: ViaModel = ViaModel {
    rows: 6,
    cols: 16,
    layout: rainy75_layout,
};

/// Rainy 75 Pro (ANSI) の配置。id = row*16+col (実機の layer0 キーマップから対応を確定)。
/// ISO/JIS 専用位置 (3,12)(4,1)(4,12) と空き位置は出さない。
fn rainy75_layout() -> Vec<LayoutKey> {
    fn k(row: u16, col: u16, x: f32, y: f32, w: f32, label: &str) -> LayoutKey {
        LayoutKey {
            id: row * 16 + col,
            x,
            y,
            w,
            h: 1.0,
            label: label.into(),
        }
    }
    let mut v = vec![k(0, 0, 0.0, 0.0, 1.0, "Esc")];
    for i in 1..=12u16 {
        v.push(k(0, i, i as f32, 0.0, 1.0, &format!("F{i}")));
    }
    v.extend([
        k(0, 13, 13.0, 0.0, 1.0, "Del"),
        k(0, 14, 15.0, 0.0, 1.0, "Home"),
        k(1, 0, 0.0, 1.0, 1.0, "`"),
    ]);
    for (i, l) in ["1", "2", "3", "4", "5", "6", "7", "8", "9", "0", "-", "="]
        .iter()
        .enumerate()
    {
        v.push(k(1, i as u16 + 1, i as f32 + 1.0, 1.0, 1.0, l));
    }
    v.extend([
        k(1, 13, 13.0, 1.0, 2.0, "Backspace"),
        k(1, 14, 15.0, 1.0, 1.0, "End"),
        k(2, 0, 0.0, 2.0, 1.5, "Tab"),
    ]);
    for (i, l) in ["Q", "W", "E", "R", "T", "Y", "U", "I", "O", "P", "[", "]"]
        .iter()
        .enumerate()
    {
        v.push(k(2, i as u16 + 1, i as f32 + 1.5, 2.0, 1.0, l));
    }
    v.extend([
        k(2, 13, 13.5, 2.0, 1.5, "\\"),
        k(3, 14, 15.0, 2.0, 1.0, "PgUp"),
        k(3, 0, 0.0, 3.0, 1.75, "Caps"),
    ]);
    for (i, l) in ["A", "S", "D", "F", "G", "H", "J", "K", "L", ";", "'"]
        .iter()
        .enumerate()
    {
        v.push(k(3, i as u16 + 1, i as f32 + 1.75, 3.0, 1.0, l));
    }
    v.extend([
        k(3, 13, 12.75, 3.0, 2.25, "Enter"),
        k(3, 15, 15.0, 3.0, 1.0, "PgDn"),
        k(4, 0, 0.0, 4.0, 2.25, "LShift"),
    ]);
    for (i, l) in ["Z", "X", "C", "V", "B", "N", "M", ",", ".", "/"]
        .iter()
        .enumerate()
    {
        v.push(k(4, i as u16 + 2, i as f32 + 2.25, 4.0, 1.0, l));
    }
    v.extend([
        k(4, 13, 12.25, 4.0, 1.75, "RShift"),
        k(4, 14, 14.0, 4.0, 1.0, "↑"),
        k(5, 0, 0.0, 5.0, 1.25, "LCtrl"),
        k(5, 1, 1.25, 5.0, 1.25, "LWin"),
        k(5, 2, 2.5, 5.0, 1.25, "LAlt"),
        k(5, 8, 3.75, 5.0, 6.25, "Space"),
        k(5, 10, 10.0, 5.0, 1.0, "Fn"),
        k(5, 11, 11.0, 5.0, 1.0, "RCtrl"),
        k(5, 13, 13.0, 5.0, 1.0, "←"),
        k(5, 14, 14.0, 5.0, 1.0, "↓"),
        k(5, 15, 15.0, 5.0, 1.0, "→"),
    ]);
    v
}

pub struct ViaDriver {
    dev: Mutex<hidapi::HidDevice>,
    info: DeviceInfo,
    model: &'static ViaModel,
    layout: Vec<LayoutKey>,
    layers: u8,
}

pub fn open(dev: hidapi::HidDevice, mut info: DeviceInfo) -> HubResult<ViaDriver> {
    dev.set_blocking_mode(false).ok();
    let model = model_for(&info.model);
    let mut d = ViaDriver {
        dev: Mutex::new(dev),
        info: info.clone(),
        model,
        layout: (model.layout)(),
        layers: 1,
    };
    let v = d.cmd(&[0x01])?;
    let proto = u16::from_be_bytes([v[1], v[2]]);
    info.firmware = Some(format!("VIA protocol {proto}"));
    d.info = info;
    d.layers = d.cmd(&[0x11]).map(|r| r[1]).unwrap_or(1).clamp(1, 16);
    Ok(d)
}

impl ViaDriver {
    /// 1 コマンド → echo の一致する応答 (最大 3 回送り直し)。
    fn cmd(&self, body: &[u8]) -> HubResult<[u8; FRAME]> {
        let dev = self.dev.lock().unwrap_or_else(|p| p.into_inner());
        let mut w = [0u8; FRAME + 1];
        w[1..1 + body.len()].copy_from_slice(body);
        // 前のコマンドの取り残しを捨てる。
        let mut junk = [0u8; FRAME + 1];
        while matches!(dev.read_timeout(&mut junk, 0), Ok(n) if n > 0) {}
        for _ in 0..3 {
            dev.write(&w)
                .map_err(|e| HubError::Io(format!("via write: {e}")))?;
            let deadline = std::time::Instant::now() + std::time::Duration::from_millis(1500);
            while std::time::Instant::now() < deadline {
                let mut r = [0u8; FRAME + 1];
                match dev.read_timeout(&mut r, 200) {
                    Ok(n) if n >= FRAME && r[0] == body[0] => {
                        let mut out = [0u8; FRAME];
                        out.copy_from_slice(&r[..FRAME]);
                        return Ok(out);
                    }
                    Ok(_) => continue,
                    Err(e) => return Err(HubError::Io(format!("via read: {e}"))),
                }
            }
        }
        Err(HubError::Protocol("via: response timeout".into()))
    }

    /// キーマップ全レイヤー分の生 buffer (big endian u16 列)。
    fn read_buffer(&self) -> HubResult<Vec<u16>> {
        let per_layer = self.model.rows as u16 * self.model.cols as u16 * 2;
        let total = per_layer * self.layers as u16;
        let mut bytes = Vec::with_capacity(total as usize);
        let mut off = 0u16;
        while off < total {
            let len = (total - off).min(CHUNK) as u8;
            let [hi, lo] = off.to_be_bytes();
            let r = self.cmd(&[0x12, hi, lo, len])?;
            bytes.extend_from_slice(&r[4..4 + len as usize]);
            off += len as u16;
        }
        Ok(bytes
            .chunks_exact(2)
            .map(|c| u16::from_be_bytes([c[0], c[1]]))
            .collect())
    }
}

// ==================================================================== キーコード

/// QMK 新体系のレイヤー系 (QK_TO / QK_MOMENTARY / QK_DEF_LAYER / QK_TOGGLE_LAYER)。
const QK_TO: u16 = 0x5200;
const QK_MO: u16 = 0x5220;
const QK_TG: u16 = 0x5260;
const QK_MACRO: u16 = 0x7700;

fn via_catalog(layers: u8) -> Vec<KeyGroup> {
    let g = |name: &str, v: Vec<(u32, String, String)>| KeyGroup {
        name: name.into(),
        keys: v
            .into_iter()
            .map(|(code, label, short)| KeyChoice {
                code,
                label,
                short: Some(short),
            })
            .collect(),
    };
    let s = |c: u32, l: &str, sh: &str| (c, l.to_string(), sh.to_string());
    let mut layer_keys = vec![];
    for i in 0..layers as u16 {
        layer_keys.push(s((QK_MO + i) as u32, &format!("押している間 レイヤー {i}"), &format!("MO{i}")));
    }
    for i in 0..layers as u16 {
        layer_keys.push(s((QK_TG + i) as u32, &format!("レイヤー {i} 切替 (トグル)"), &format!("TG{i}")));
    }
    for i in 0..layers as u16 {
        layer_keys.push(s((QK_TO + i) as u32, &format!("レイヤー {i} へ移動"), &format!("TO{i}")));
    }
    vec![
        devhub_core::keys::basic_group(),
        g(
            "マウス",
            vec![
                s(0xD1, "マウス左", "M左"),
                s(0xD2, "マウス右", "M右"),
                s(0xD3, "マウス中", "M中"),
                s(0xD4, "マウス戻る", "M戻"),
                s(0xD5, "マウス進む", "M進"),
                s(0xD9, "ホイール上", "W↑"),
                s(0xDA, "ホイール下", "W↓"),
            ],
        ),
        g(
            "メディア",
            vec![
                s(0xAE, "再生/一時停止", "⏯"),
                s(0xAB, "次の曲", "⏭"),
                s(0xAC, "前の曲", "⏮"),
                s(0xAD, "停止", "⏹"),
                s(0xA9, "音量 +", "🔊"),
                s(0xAA, "音量 −", "🔉"),
                s(0xA8, "ミュート", "🔇"),
            ],
        ),
        g("レイヤー", layer_keys),
    ]
}

/// QMK 修飾 (bit0 Ctrl,1 Shift,2 Alt,3 GUI, bit4=右) → HID 修飾ビット。
fn qmk_mods_to_hid(m: u8) -> u8 {
    let low = m & 0x0F;
    if m & 0x10 != 0 {
        low << 4
    } else {
        low
    }
}

/// HID 修飾ビット → QMK 修飾 (左右混在は左に寄せる)。
fn hid_mods_to_qmk(m: u8) -> u8 {
    let left = m & 0x0F;
    let right = m >> 4;
    if left != 0 || right == 0 {
        left | right
    } else {
        0x10 | right
    }
}

fn code_to_action(c: u16) -> KeyAction {
    match c {
        0x0000 => KeyAction::None,
        0x0001 => KeyAction::Transparent,
        // QK_MODS: mods<<8 | basic keycode
        0x0100..=0x1FFF => KeyAction::Combo {
            mods: qmk_mods_to_hid((c >> 8) as u8),
            code: (c & 0xFF) as u32,
        },
        c if (QK_MACRO..QK_MACRO + 0x80).contains(&c) => KeyAction::Macro((c - QK_MACRO) as u8),
        c => KeyAction::Code(c as u32),
    }
}

fn action_to_code(a: &KeyAction) -> HubResult<u16> {
    Ok(match a {
        KeyAction::None => 0,
        KeyAction::Transparent => 1,
        KeyAction::Code(c) => u16::try_from(*c).map_err(|_| HubError::Invalid("keycode".into()))?,
        KeyAction::Combo { mods, code } => {
            if *code > 0xFF {
                return Err(HubError::Invalid("combo keycode".into()));
            }
            let m = hid_mods_to_qmk(*mods);
            if m == 0 {
                *code as u16
            } else {
                ((m as u16) << 8) | *code as u16
            }
        }
        KeyAction::Macro(i) if *i < 0x80 => QK_MACRO + *i as u16,
        KeyAction::Macro(_) => return Err(HubError::Invalid("macro slot".into())),
    })
}

// ==================================================================== DeviceDriver

impl DeviceDriver for ViaDriver {
    fn info(&self) -> DeviceInfo {
        let mut i = self.info.clone();
        i.supported = true;
        i
    }

    fn caps(&self) -> Capabilities {
        Capabilities {
            settings: vec![],
            actions: vec![],
            profiles: None,
            mouse: None,
            keyboard: Some(KeyboardCaps {
                layout: self.layout.clone(),
                layers: (0..self.layers)
                    .map(|i| {
                        if i == 0 {
                            "メイン".into()
                        } else {
                            format!("レイヤー {i}")
                        }
                    })
                    .collect(),
                catalog: via_catalog(self.layers),
                combos: true,
                perf: None,
                advanced: vec![],
                macros: None,
            }),
            lighting: vec![],
            battery: false,
            peq: None,
            firmware_feed: None,
        }
    }

    fn read(&self) -> HubResult<DeviceState> {
        let buf = self.read_buffer()?;
        let per_layer = self.model.rows as usize * self.model.cols as usize;
        let keymap = (0..self.layers as usize)
            .map(|l| {
                self.layout
                    .iter()
                    .map(|k| {
                        buf.get(l * per_layer + k.id as usize)
                            .map(|&c| code_to_action(c))
                            .unwrap_or(KeyAction::None)
                    })
                    .collect()
            })
            .collect();
        Ok(DeviceState {
            profile: ProfileState {
                active: 0,
                names: vec![],
            },
            settings: BTreeMap::new(),
            mouse: None,
            keyboard: Some(KeyboardState {
                keymap,
                perf: vec![],
                advanced: vec![],
                macros: vec![],
            }),
            lighting: BTreeMap::new(),
            peq: None,
        })
    }

    fn apply(&self, change: &Change) -> HubResult<()> {
        match change {
            Change::KeyMap { layer, key, action } => {
                if *layer >= self.layers {
                    return Err(HubError::Invalid("layer".into()));
                }
                let k = self
                    .layout
                    .get(*key)
                    .ok_or_else(|| HubError::Invalid("key".into()))?;
                let row = (k.id / self.model.cols as u16) as u8;
                let col = (k.id % self.model.cols as u16) as u8;
                let [hi, lo] = action_to_code(action)?.to_be_bytes();
                let r = self.cmd(&[0x05, *layer, row, col, hi, lo])?;
                // 書いた値が echo される (ファーム側で拒否されたら 0xFF が返る実装もある)。
                if r[1] == 0xFF {
                    return Err(HubError::Protocol("via: set_keycode rejected".into()));
                }
                Ok(())
            }
            _ => Err(HubError::Unsupported("change")),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keycodes_roundtrip() {
        assert_eq!(code_to_action(0), KeyAction::None);
        assert_eq!(code_to_action(1), KeyAction::Transparent);
        assert_eq!(code_to_action(0x29), KeyAction::Code(0x29));
        assert_eq!(code_to_action(0x5221), KeyAction::Code(0x5221));
        // Ctrl+C (QMK LCTL(KC_C) = 0x0106)
        let a = code_to_action(0x0106);
        assert_eq!(a, KeyAction::Combo { mods: 0x01, code: 0x06 });
        assert_eq!(action_to_code(&a).unwrap(), 0x0106);
        // RShift+A (QMK RSFT(KC_A) = 0x1204) → HID RShift bit5
        let a = code_to_action(0x1204);
        assert_eq!(a, KeyAction::Combo { mods: 0x20, code: 0x04 });
        assert_eq!(action_to_code(&a).unwrap(), 0x1204);
    }

    #[test]
    fn layout_ids_unique_and_in_matrix() {
        let l = rainy75_layout();
        let mut ids: Vec<u16> = l.iter().map(|k| k.id).collect();
        ids.sort();
        ids.dedup();
        assert_eq!(ids.len(), l.len());
        assert!(ids.iter().all(|&i| i < 6 * 16));
    }
}
