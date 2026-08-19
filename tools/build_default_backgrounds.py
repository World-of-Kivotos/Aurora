#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""
生成 frost / liquid 两个视觉模式的内置默认背景图（占位美术）。

这两张图是「刚装好、还没导入自己壁纸」时的兜底底图，不是最终成品美术——
真正的判据在 docs/Glass_Redesign_AssetChecklist.md，人类换图时必须过那份清单。
本脚本同时把清单里「右下角安静区」那条硬约束的量法实现成可执行代码，
而不是只在文档里摆公式：生成完立刻自测，任何一张不达标就直接报错退出，
不允许「先生成、回头再肉眼看着办」。

安静区判据原本与 app/src/lib/appearance.ts 的 plateMode/effectiveLuma 逐系数对齐。
2026-08-19 换皮收口时前端那套已整体删除（主页右下角的字改坐 .surface-panel，不再裸压照片），
所以下面这份实现现在是孤本，不再有需要同步的对端。它仍然能跑、判定口径不变，
但通过与否只说明「右下角明暗跨度大不大」，不再对应任何运行时渲染分支——
详见 docs/Glass_Redesign_AssetChecklist.md 第三节开头的作废说明。

用法：
    python tools/build_default_backgrounds.py
        生成 frost.png 与 liquid.png 到 app/src/assets/backgrounds/，并打印安静区自测结果。

    python tools/build_default_backgrounds.py --measure <图片路径>
        只对一张已有图片跑安静区判定，不生成任何文件。人类挑选替换图后用这个先自查，
        比等应用里导入了才看结果快。
"""

from __future__ import annotations

import argparse
import math
import sys
from pathlib import Path

from PIL import Image, ImageDraw, ImageFilter

if sys.stdout.encoding is not None and sys.stdout.encoding.lower() != "utf-8":
    # Windows 终端默认走系统代码页（如 GBK），直接 print 中文会花屏；改道 UTF-8。
    sys.stdout.reconfigure(encoding="utf-8")
    sys.stderr.reconfigure(encoding="utf-8")

# ----------------------------------------------------------------------------
# 一、与 appearance.ts 对齐的常量与换算（改这里之前先改那边，反之亦然）
# ----------------------------------------------------------------------------

CANVAS_SIZE = (1920, 1080)

# 安静区：右下角，占 28% 宽 / 34% 高。与 app.css 的 .plate-scrim 遮罩、
# Home.tsx 的信息卡片摆位是同一块区域。
QUIET_ZONE_WIDTH_FRAC = 0.28
QUIET_ZONE_HEIGHT_FRAC = 0.34

PAPER_SRGB = 242.0
L_INK = 0.007971
L_PAPER_ON = 0.905855
SCRIM_BRIGHTNESS = 0.5  # 必须与 appearance.ts 的 SCRIM_BRIGHTNESS 是同一个数
NAKED_TARGET = 5.5

# 硬阈值是刚好卡过 NAKED_TARGET 的分界点，设计时不能贴着它走——
# 压缩/重采样/肉眼与算法取样点的一点点误差就会把结果推过界。留出的余量：
#   ink   要求 p10 >= 69，设计目标定在 100（多留 31）
#   paperOn 要求 p90 <= 142，设计目标定在 115（多留 27）
#   两端极差（p90 - p10）额外限制在 45 以内：防止「均值达标但一小撮高光/暗影单独出格」。
INK_HARD_MIN_P10 = 69
PAPER_ON_HARD_MAX_P90 = 142
DESIGN_MARGIN_P10_MIN = 100
DESIGN_MARGIN_P90_MAX = 115
DESIGN_MARGIN_SPREAD_MAX = 45


def luma_of(srgb: float) -> float:
    """sRGB 灰阶（0..255）转相对亮度（0..1）。与 appearance.ts 的 lumaOf 同一公式。"""
    c = srgb / 255.0
    return c / 12.92 if c <= 0.04045 else ((c + 0.055) / 1.055) ** 2.4


def srgb_of(luma: float) -> float:
    """相对亮度（0..1）还原成等效 sRGB 灰阶（0..255）。lumaOf 的逆函数。"""
    c = luma * 12.92 if luma <= 0.0031308 else 1.055 * luma ** (1 / 2.4) - 0.055
    return c * 255.0


def pixel_relative_luminance(r: int, g: int, b: int) -> float:
    """WCAG 相对亮度：逐通道线性化后按 0.2126/0.7152/0.0722 加权。appearance.ts 里存的
    plate.p10/p90 就是这个量的百分位数，再线性映射到 0..255 字节（ipc.ts 的接口注释原话）。"""

    def linearize(c8: int) -> float:
        c = c8 / 255.0
        return c / 12.92 if c <= 0.04045 else ((c + 0.055) / 1.055) ** 2.4

    return 0.2126 * linearize(r) + 0.7152 * linearize(g) + 0.0722 * linearize(b)


def percentile_nearest_rank(sorted_values: list[float], q: float) -> float:
    """最近秩百分位数。这是近似判据——后端 Rust 侧实际用的插值方法本仓库的前端职责范围内
    看不到源码，量图时按这个口径留够设计余量（见上面 DESIGN_MARGIN_*），
    单像素级别的算法差异不足以吃掉这么大的余量。"""
    if not sorted_values:
        raise ValueError("空取样集合")
    idx = round(q * (len(sorted_values) - 1))
    return sorted_values[idx]


def quiet_zone_rect(size: tuple[int, int]) -> tuple[int, int, int, int]:
    """右下角安静区的像素矩形（左, 上, 右, 下），右/下贴画布边缘。"""
    w, h = size
    zone_w = round(w * QUIET_ZONE_WIDTH_FRAC)
    zone_h = round(h * QUIET_ZONE_HEIGHT_FRAC)
    return (w - zone_w, h - zone_h, w, h)


def measure_quiet_zone(img: Image.Image) -> dict:
    """对一张图的安静区跑取样，返回 p10/p90（0..255 尺度，对齐 PlateZone 存储口径）与分类结果。"""
    rgb = img.convert("RGB")
    box = quiet_zone_rect(rgb.size)
    crop = rgb.crop(box)
    pixels = crop.get_flattened_data()  # 逐像素 (r,g,b) 元组；getdata() 在 Pillow 12 已弃用
    lumas = sorted(pixel_relative_luminance(r, g, b) * 255.0 for r, g, b in pixels)

    p10 = percentile_nearest_rank(lumas, 0.10)
    p90 = percentile_nearest_rank(lumas, 0.90)
    spread = p90 - p10

    dark_end = luma_of(srgb_of(p10 / 255.0) * 1.0)  # veil=0, dim=1 时 effectiveLuma 恒等于 p10/255
    ink_contrast = (dark_end + 0.05) / (L_INK + 0.05)

    bright_end_raw = srgb_of(p90 / 255.0) * SCRIM_BRIGHTNESS
    bright_end = luma_of(bright_end_raw)
    paper_on_contrast = (L_PAPER_ON + 0.05) / (bright_end + 0.05)

    if ink_contrast >= NAKED_TARGET:
        mode = "ink"
    elif paper_on_contrast >= NAKED_TARGET:
        mode = "paperOn"
    else:
        mode = "plate"

    within_margin = (
        mode == "paperOn"
        and p90 <= DESIGN_MARGIN_P90_MAX
        and spread <= DESIGN_MARGIN_SPREAD_MAX
    ) or (
        mode == "ink"
        and p10 >= DESIGN_MARGIN_P10_MIN
        and spread <= DESIGN_MARGIN_SPREAD_MAX
    )

    return {
        "box": box,
        "p10": p10,
        "p90": p90,
        "spread": spread,
        "ink_contrast": ink_contrast,
        "paper_on_contrast": paper_on_contrast,
        "mode": mode,
        "within_design_margin": within_margin,
    }


def report_measurement(label: str, m: dict) -> None:
    print(f"[{label}] 安静区矩形 = {m['box']}")
    print(
        f"[{label}] p10={m['p10']:.1f} p90={m['p90']:.1f} 极差={m['spread']:.1f} "
        f"(0..255 尺度)"
    )
    print(
        f"[{label}] ink 对比度={m['ink_contrast']:.2f} (>=5.5 通过) / "
        f"paperOn 对比度={m['paper_on_contrast']:.2f} (>=5.5 通过)"
    )
    print(f"[{label}] 判定分支 = {m['mode']} / 落在设计余量内 = {m['within_design_margin']}")


# ----------------------------------------------------------------------------
# 二、生图：平滑渐变 + 少量柔化色块，刻意避免高频细节（原因见资产清单文档）
# ----------------------------------------------------------------------------


def vertical_gradient(size: tuple[int, int], top: tuple[int, int, int], bottom: tuple[int, int, int]) -> Image.Image:
    w, h = size
    img = Image.new("RGB", size)
    draw = ImageDraw.Draw(img)
    for y in range(h):
        t = y / (h - 1)
        row = tuple(round(top[i] + (bottom[i] - top[i]) * t) for i in range(3))
        draw.line([(0, y), (w, y)], fill=row)
    return img


def soft_blob(
    size: tuple[int, int],
    center: tuple[int, int],
    radii: tuple[int, int],
    color: tuple[int, int, int],
    alpha: int,
    blur: int,
) -> Image.Image:
    """一枚柔化圆斑，用大半径高斯模糊避免任何硬边/高频轮廓。"""
    layer = Image.new("RGBA", size, (0, 0, 0, 0))
    draw = ImageDraw.Draw(layer)
    cx, cy = center
    rx, ry = radii
    draw.ellipse([cx - rx, cy - ry, cx + rx, cy + ry], fill=(*color, alpha))
    return layer.filter(ImageFilter.GaussianBlur(blur))


def diagonal_sheen(
    size: tuple[int, int],
    band_size: tuple[int, int],
    color: tuple[int, int, int],
    alpha: int,
    angle: float,
    center: tuple[int, int],
    blur: int,
) -> Image.Image:
    """一道柔化的斜向高光带，呼应毛玻璃/液态玻璃边缘受光的意象——同样重度模糊，不留硬边。"""
    bw, bh = band_size
    band = Image.new("RGBA", (bw, bh), (0, 0, 0, 0))
    ImageDraw.Draw(band).rectangle([0, 0, bw, bh], fill=(*color, alpha))
    band = band.filter(ImageFilter.GaussianBlur(blur))
    rotated = band.rotate(angle, expand=True, resample=Image.BICUBIC)

    layer = Image.new("RGBA", size, (0, 0, 0, 0))
    px = center[0] - rotated.width // 2
    py = center[1] - rotated.height // 2
    layer.alpha_composite(rotated, (px, py))
    return layer


def quiet_zone_patch(
    size: tuple[int, int],
    color: tuple[int, int, int],
    alpha: int,
    blur: int,
) -> Image.Image:
    """压在最上层、专门抹平右下角安静区的柔化色块。

    覆盖范围刻意比安静区矩形大一圈再做重度模糊，让「平坦区」完整盖住安静区矩形本身，
    矩形边界落在渐变的柔和过渡段之外——否则矩形边缘正好切在模糊衰减带上，
    会在安静区内部量出一道人为的假极差。
    """
    w, h = size
    zone_l, zone_t, zone_r, zone_b = quiet_zone_rect(size)
    margin = blur  # 让平坦区外扩量至少覆盖一个模糊半径，衰减带才不会啃进矩形内部
    cx = zone_r - (zone_r - zone_l) // 2
    cy = zone_b - (zone_b - zone_t) // 2
    rx = (zone_r - zone_l) // 2 + margin
    ry = (zone_b - zone_t) // 2 + margin

    layer = Image.new("RGBA", size, (0, 0, 0, 0))
    draw = ImageDraw.Draw(layer)
    draw.ellipse([cx - rx * 1.6, cy - ry * 1.6, cx + rx * 1.6, cy + ry * 1.6], fill=(*color, alpha))
    return layer.filter(ImageFilter.GaussianBlur(blur))


def build_frost() -> Image.Image:
    """frost 模式：冷色调，呼应「结霜的毛玻璃」——浅冰蓝到石板蓝灰的竖向渐变。"""
    base = vertical_gradient(CANVAS_SIZE, top=(214, 226, 233), bottom=(88, 103, 120)).convert("RGBA")

    glow = soft_blob(CANVAS_SIZE, center=(1440, 220), radii=(360, 250), color=(238, 248, 250), alpha=150, blur=130)
    base = Image.alpha_composite(base, glow)

    deep = soft_blob(CANVAS_SIZE, center=(300, 820), radii=(420, 300), color=(46, 64, 82), alpha=115, blur=150)
    base = Image.alpha_composite(base, deep)

    sheen = diagonal_sheen(
        CANVAS_SIZE,
        band_size=(1500, 180),
        color=(255, 255, 255),
        alpha=30,
        angle=-24,
        center=(760, 420),
        blur=140,
    )
    base = Image.alpha_composite(base, sheen)

    patch = quiet_zone_patch(CANVAS_SIZE, color=(55, 66, 80), alpha=190, blur=170)
    base = Image.alpha_composite(base, patch)

    return base.convert("RGB")


def build_liquid() -> Image.Image:
    """liquid 模式：暖色调，呼应「液态玻璃」的金属光泽——暖米到深青的竖向渐变。"""
    base = vertical_gradient(CANVAS_SIZE, top=(247, 228, 198), bottom=(35, 90, 92)).convert("RGBA")

    gold = soft_blob(CANVAS_SIZE, center=(1460, 200), radii=(360, 250), color=(255, 214, 150), alpha=140, blur=140)
    base = Image.alpha_composite(base, gold)

    teal = soft_blob(CANVAS_SIZE, center=(300, 840), radii=(440, 310), color=(52, 122, 116), alpha=120, blur=160)
    base = Image.alpha_composite(base, teal)

    sheen = diagonal_sheen(
        CANVAS_SIZE,
        band_size=(1500, 190),
        color=(255, 246, 224),
        alpha=32,
        angle=-22,
        center=(720, 380),
        blur=150,
    )
    base = Image.alpha_composite(base, sheen)

    patch = quiet_zone_patch(CANVAS_SIZE, color=(26, 56, 62), alpha=195, blur=175)
    base = Image.alpha_composite(base, patch)

    return base.convert("RGB")


# ----------------------------------------------------------------------------
# 三、主流程
# ----------------------------------------------------------------------------

OUTPUT_DIR = Path(__file__).resolve().parent.parent / "app" / "src" / "assets" / "backgrounds"


def build_and_verify() -> None:
    OUTPUT_DIR.mkdir(parents=True, exist_ok=True)

    jobs = {"frost": build_frost, "liquid": build_liquid}
    failures: list[str] = []

    for label, builder in jobs.items():
        img = builder()
        assert img.size == CANVAS_SIZE, f"{label} 尺寸不是 {CANVAS_SIZE}: {img.size}"

        out_path = OUTPUT_DIR / f"{label}.png"
        img.save(out_path, format="PNG")
        print(f"已生成 {out_path}")

        m = measure_quiet_zone(img)
        report_measurement(label, m)
        if not m["within_design_margin"]:
            failures.append(label)
        print()

    if failures:
        print(f"未达设计余量的图：{failures}，请调整对应 build_* 里的色值/alpha/blur 后重跑。", file=sys.stderr)
        sys.exit(1)

    print("两张占位背景均已生成，安静区判定均落在设计余量内。")


def measure_only(path_str: str) -> None:
    path = Path(path_str)
    if not path.is_file():
        print(f"文件不存在：{path}", file=sys.stderr)
        sys.exit(1)
    img = Image.open(path)
    if img.size != CANVAS_SIZE:
        print(
            f"警告：{path.name} 尺寸为 {img.size}，不是标准的 {CANVAS_SIZE}；"
            f"安静区矩形的换算会跟着按等比例走，量出来的结论仅供参考。"
        )
    m = measure_quiet_zone(img)
    report_measurement(path.name, m)
    if not m["within_design_margin"]:
        print(f"{path.name} 未落在设计余量内（硬阈值仍可能过，但没有安全余量）。", file=sys.stderr)
        sys.exit(1)


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "--measure",
        metavar="图片路径",
        help="只对指定图片跑安静区判定，不生成任何文件。",
    )
    args = parser.parse_args()

    if args.measure:
        measure_only(args.measure)
    else:
        build_and_verify()


if __name__ == "__main__":
    main()
