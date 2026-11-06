// 应用图标：唯一来源。窗口、任务栏、托盘、exe 文件图标、安装包图标都从这里画出来。
//
// 这个文件也被 build.rs 用 include! 引入（编译时生成 exe 的图标资源），
// 所以不能引用 crate:: 里的东西，也不能用外部依赖。

#[allow(dead_code)]
const ICON_INK: [u8; 3] = [0x17, 0x18, 0x1a];

/// exe 图标里放的尺寸（资源管理器、开始菜单、任务栏在不同缩放下各取所需）
#[allow(dead_code)]
pub const ICON_SIZES: [u32; 8] = [16, 20, 24, 32, 40, 48, 64, 256];

/// 墨黑圆角方块 + 白色声波条。直接画出来，不用另带图片文件。返回 RGBA（非预乘）。
#[allow(dead_code)]
pub fn app_icon_rgba(size: u32) -> Vec<u8> {
    let s = size as f32;
    let radius = s * 0.24;
    // 五根竖条，高度对称起伏
    let heights = [0.26f32, 0.46, 0.62, 0.46, 0.26];
    let bar_w = s * 0.09;
    let gap = s * 0.06;
    let total = heights.len() as f32 * bar_w + (heights.len() - 1) as f32 * gap;
    let x0 = (s - total) / 2.0;
    let inside_rect = |x: f32, y: f32| {
        let cx = x.clamp(radius, s - radius);
        let cy = y.clamp(radius, s - radius);
        (x - cx).powi(2) + (y - cy).powi(2) <= radius * radius
    };
    let inside_bar = |x: f32, y: f32| {
        heights.iter().enumerate().any(|(i, h)| {
            let r = bar_w / 2.0;
            let cx = x0 + i as f32 * (bar_w + gap) + r;
            let half = (h * s / 2.0 - r).max(0.0);
            let dy = ((y - s / 2.0).abs() - half).max(0.0);
            (x - cx).powi(2) + dy * dy <= r * r
        })
    };
    const SS: usize = 4; // 每像素 4×4 采样做抗锯齿
    let mut out = Vec::with_capacity((size * size * 4) as usize);
    for py in 0..size {
        for px in 0..size {
            let (mut a, mut white) = (0usize, 0usize);
            for sy in 0..SS {
                for sx in 0..SS {
                    let x = px as f32 + (sx as f32 + 0.5) / SS as f32;
                    let y = py as f32 + (sy as f32 + 0.5) / SS as f32;
                    if inside_rect(x, y) {
                        a += 1;
                        if inside_bar(x, y) {
                            white += 1;
                        }
                    }
                }
            }
            if a == 0 {
                out.extend_from_slice(&[0, 0, 0, 0]);
                continue;
            }
            let t = white as f32 / a as f32;
            let mix = |ink: u8| (ink as f32 + (255.0 - ink as f32) * t).round() as u8;
            out.extend_from_slice(&[
                mix(ICON_INK[0]),
                mix(ICON_INK[1]),
                mix(ICON_INK[2]),
                (a * 255 / (SS * SS)) as u8,
            ]);
        }
    }
    out
}

/// 一张图标图像（ICO 文件和 exe 的 RT_ICON 资源里是同一种格式）：
/// BITMAPINFOHEADER（高度写两倍）+ 自下而上的 BGRA 像素 + 全 0 的 1 位掩码（透明靠 alpha）
#[allow(dead_code)]
pub fn icon_dib(size: u32) -> Vec<u8> {
    let rgba = app_icon_rgba(size);
    let mask_row = ((size + 31) / 32 * 4) as usize;
    let mut out = Vec::with_capacity(40 + rgba.len() + mask_row * size as usize);
    out.extend_from_slice(&40u32.to_le_bytes());
    out.extend_from_slice(&(size as i32).to_le_bytes());
    out.extend_from_slice(&((size * 2) as i32).to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes());
    out.extend_from_slice(&32u16.to_le_bytes());
    out.extend_from_slice(&[0u8; 24]);
    for y in (0..size).rev() {
        for x in 0..size {
            let i = ((y * size + x) * 4) as usize;
            out.extend_from_slice(&[rgba[i + 2], rgba[i + 1], rgba[i], rgba[i + 3]]);
        }
    }
    out.resize(out.len() + mask_row * size as usize, 0);
    out
}

/// 完整的 .ico 文件（安装包图标用）
#[allow(dead_code)]
pub fn ico_file() -> Vec<u8> {
    let images: Vec<(u32, Vec<u8>)> = ICON_SIZES.iter().map(|&s| (s, icon_dib(s))).collect();
    let mut out = Vec::new();
    out.extend_from_slice(&0u16.to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes());
    out.extend_from_slice(&(images.len() as u16).to_le_bytes());
    let mut offset = 6 + 16 * images.len() as u32;
    for (s, img) in &images {
        let dim = if *s >= 256 { 0 } else { *s as u8 };
        out.extend_from_slice(&[dim, dim, 0, 0]);
        out.extend_from_slice(&1u16.to_le_bytes());
        out.extend_from_slice(&32u16.to_le_bytes());
        out.extend_from_slice(&(img.len() as u32).to_le_bytes());
        out.extend_from_slice(&offset.to_le_bytes());
        offset += img.len() as u32;
    }
    for (_, img) in &images {
        out.extend_from_slice(img);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn icon_shape() {
        let n = 32u32;
        let px = app_icon_rgba(n);
        assert_eq!(px.len(), (n * n * 4) as usize);
        let at = |x: u32, y: u32| &px[((y * n + x) * 4) as usize..((y * n + x) * 4 + 4) as usize];
        assert_eq!(at(0, 0)[3], 0, "圆角外透明");
        assert_eq!(at(n / 2, n / 2)[0], 255, "中间是白色声波条");
        assert_eq!(at(3, n / 2)[3], 255, "边上是不透明的墨色底");
    }

    #[test]
    fn ico_layout() {
        let ico = ico_file();
        assert_eq!(&ico[..6], &[0, 0, 1, 0, ICON_SIZES.len() as u8, 0]);
        // 每个目录项的偏移 + 长度都落在文件内，且指向 BITMAPINFOHEADER
        for i in 0..ICON_SIZES.len() {
            let e = &ico[6 + i * 16..6 + i * 16 + 16];
            let len = u32::from_le_bytes(e[8..12].try_into().unwrap()) as usize;
            let off = u32::from_le_bytes(e[12..16].try_into().unwrap()) as usize;
            assert!(off + len <= ico.len());
            assert_eq!(&ico[off..off + 4], &40u32.to_le_bytes());
        }
    }
}
