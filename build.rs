//! 把应用图标编进 exe（资源管理器、开始菜单、桌面快捷方式显示的就是它）。
//! 图标由 src/icon.rs 画出，这里直接写 .res 资源文件交给 MSVC 链接器，不依赖 rc.exe。

include!("src/icon.rs");

fn main() {
    println!("cargo:rerun-if-changed=src/icon.rs");
    println!("cargo:rerun-if-changed=build.rs");
    let target = std::env::var("TARGET").unwrap_or_default();
    if !target.contains("windows-msvc") {
        return;
    }
    let out = std::path::PathBuf::from(std::env::var("OUT_DIR").expect("OUT_DIR"));
    let res = out.join("app_icon.res");
    std::fs::write(&res, icon_res()).expect("写 app_icon.res 失败");
    println!("cargo:rustc-link-arg-bins={}", res.display());
}

const RT_ICON: u16 = 3;
const RT_GROUP_ICON: u16 = 14;
const LANG_EN_US: u16 = 0x0409;

/// .res 里的一条资源（类型、名字都用数字 ID）
fn res_entry(out: &mut Vec<u8>, kind: u16, id: u16, flags: u16, data: &[u8]) {
    out.extend_from_slice(&(data.len() as u32).to_le_bytes()); // DataSize
    out.extend_from_slice(&32u32.to_le_bytes()); // HeaderSize
    out.extend_from_slice(&0xFFFFu16.to_le_bytes());
    out.extend_from_slice(&kind.to_le_bytes());
    out.extend_from_slice(&0xFFFFu16.to_le_bytes());
    out.extend_from_slice(&id.to_le_bytes());
    out.extend_from_slice(&0u32.to_le_bytes()); // DataVersion
    out.extend_from_slice(&flags.to_le_bytes()); // MemoryFlags
    out.extend_from_slice(&(if kind == 0 { 0 } else { LANG_EN_US }).to_le_bytes());
    out.extend_from_slice(&0u32.to_le_bytes()); // Version
    out.extend_from_slice(&0u32.to_le_bytes()); // Characteristics
    out.extend_from_slice(data);
    while out.len() % 4 != 0 {
        out.push(0);
    }
}

fn icon_res() -> Vec<u8> {
    let mut out = Vec::new();
    // .res 文件约定以一条空资源开头
    res_entry(&mut out, 0, 0, 0, &[]);
    let mut group = Vec::new();
    group.extend_from_slice(&0u16.to_le_bytes());
    group.extend_from_slice(&1u16.to_le_bytes());
    group.extend_from_slice(&(ICON_SIZES.len() as u16).to_le_bytes());
    for (i, &s) in ICON_SIZES.iter().enumerate() {
        let id = i as u16 + 1;
        let img = icon_dib(s);
        let dim = if s >= 256 { 0 } else { s as u8 };
        group.extend_from_slice(&[dim, dim, 0, 0]);
        group.extend_from_slice(&1u16.to_le_bytes());
        group.extend_from_slice(&32u16.to_le_bytes());
        group.extend_from_slice(&(img.len() as u32).to_le_bytes());
        group.extend_from_slice(&id.to_le_bytes());
        res_entry(&mut out, RT_ICON, id, 0x1010, &img);
    }
    // ID 最小的图标组就是 exe 的图标
    res_entry(&mut out, RT_GROUP_ICON, 1, 0x1030, &group);
    out
}
