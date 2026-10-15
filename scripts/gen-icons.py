#!/usr/bin/env python3
"""生成应用图标源文件（SVG），再由 `just icons` 调用 tauri icon 栅格化到各平台。

设计说明
--------
标记 = **云 + 环绕数据流**：一朵数据云，一条环绕云体的轨道环，环上一道掠过的
光带，光带下方跟着一串下落的 0/1 数据位。语义直接对上"运维综合平台"：
云是数据云 / 云管平台，0/1 是上下行的数据流，环是设备与平台之间的链路。

几条刻意的取舍（都是踩过的坑）：
- **不用"云 + 竖直等长线条"**：那是"下雨"，语义会跑偏；数据位必须与云体分离、
  并有明暗渐变，才读作"数据在流动"。
- **0/1 不用字体、画成矢量字形**：不依赖系统字体（resvg 栅格化时找不到字体会
  退化成方块），任何尺寸都锐利；且**保持直立**——顺着环的切线旋转会让环底的
  数字上下颠倒。
- **数据位与光带并行、不相交**：压在光带上会被切断，两层信息互相干扰。
- 小尺寸是自然降级：0/1 在 32px 缩成点缀，云 + 光环的轮廓仍然成立。
- 没有任何文字，也没有扳手 / 齿轮 / 盾牌这类行业陈词滥调。

几何（1024 画布）
- 圆角方底：Apple macOS Big Sur 起的图标网格 —— 主体 824×824 居中（四周留 100），
  超椭圆（superellipse, n=5）拟合 Apple 的连续圆角，而不是普通圆角矩形。
- tauri icon 不做任何遮罩，圆角必须画进源图（Windows / Linux 同样吃这套几何）。
- Windows 11 系统圆角约 8px（32px 图标尺度），与本几何的圆角比例接近，视觉一致。

色板
- 底：Linear #1C2C4A → #0A101C（与应用 --bg #0B1120 同族）
- 云体：Linear #5B7FA6 → #16253A（上受光、下沉入暗部，制造体积）
- 主色：cyan #00E5FF（应用 accent，保持一致）
"""

import json
import math
import os
import shutil
import subprocess
import sys
import tempfile

CANVAS = 1024
BODY = 824                      # 圆角方底边长（Apple 图标网格：100 / 824 / 100）
INSET = (CANVAS - BODY) / 2
SQUIRCLE_N = 5.0                # 超椭圆指数，拟合 Apple 连续圆角
CX = CY = CANVAS / 2
TRAY_PX = 44                    # 托盘 PNG 边长：22pt @2x，macOS 菜单栏图标的标准尺寸

# --- 云体几何：一个圆角底座 + 若干圆穹，用同一渐变填充 → 并集无缝 ---
CLOUD_BASE = dict(x0=190, x1=834, y0=452, y1=622, r=104)
CLOUD_LOBES = [
    (512, 400, 200),
    (338, 472, 148),
    (686, 468, 152),
    (262, 546, 86),
    (762, 544, 86),
]
CLOUD_EDGE = 7                  # 云体轮廓光的外扩量
CLOUD_BOTTOM = 626              # 数据位要落在这条线之下，避免压在云体上

# --- 环绕数据环 ---
RING_CX, RING_CY = 512, 606
RING_RX, RING_RY, RING_ROT = 394, 124, -12
RING_STROKE = 20
ARC_T0, ARC_T1 = 0.62, 1.64     # 光带的角度区间（屏幕下方那段长弧）
ARC_STROKE = 24

# --- 数据位 0/1 ---
BIT_X = [318, 415, 512, 609, 706]   # 按 x 等距铺开（椭圆参数 ≠ 弧长，等分会挤在中间）
BIT_SEQ = ('1', '0', '1', '0', '1')  # 头端（右）最亮，向左渐隐
BIT_DY = 30                          # 相对环线的下移量：与光带并行、不相交
BIT_H, BIT_W, BIT_SW = 54, 35, 15    # 字形半高 / 半宽 / 笔画宽

REPO = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
ICONS = os.path.join(REPO, "src-tauri", "icons")


def squircle_path(size, cx=CX, cy=CY, n=SQUIRCLE_N, steps=288):
    """超椭圆路径：|x/a|^n + |y/a|^n = 1，用密集折线拟合，栅格化后看不出棱角。"""
    a = size / 2.0
    pts = []
    for i in range(steps):
        t = 2 * math.pi * i / steps
        ct, st = math.cos(t), math.sin(t)
        x = a * math.copysign(abs(ct) ** (2.0 / n), ct)
        y = a * math.copysign(abs(st) ** (2.0 / n), st)
        pts.append((cx + x, cy + y))
    d = "M " + " L ".join(f"{x:.2f} {y:.2f}" for x, y in pts) + " Z"
    return d


def run(cmd):
    print("$", " ".join(os.path.basename(c) if i == 0 else c for i, c in enumerate(cmd)))
    subprocess.run(cmd, check=True, cwd=REPO)


# --- 环：把角度映射到环绕椭圆的坐标 ---
_RING_TH = math.radians(RING_ROT)


def ring_pt(t):
    x, y = RING_RX * math.cos(t), RING_RY * math.sin(t)
    return (RING_CX + x * math.cos(_RING_TH) - y * math.sin(_RING_TH),
            RING_CY + x * math.sin(_RING_TH) + y * math.cos(_RING_TH))


def ring_arc_path(t0, t1, steps=72):
    pts = [ring_pt(t0 + i * (t1 - t0) / steps) for i in range(steps + 1)]
    return "M " + " L ".join(f"{x:.1f} {y:.1f}" for x, y in pts)


def bits_on_bottom(target_x):
    """在环的下半段按目标 x 取点：椭圆参数 t 与弧长不成正比，必须按 x 抽样"""
    cand = []
    for i in range(2000):
        x, y = ring_pt(2 * math.pi * i / 2000)
        if y > CLOUD_BOTTOM + 6:
            cand.append((x, y))
    return [min(cand, key=lambda p: abs(p[0] - tx)) for tx in target_x]


def bit_glyph(kind, x, y, opacity):
    """0 / 1 的矢量字形：不依赖字体，直立摆放"""
    h, w, sw = BIT_H, BIT_W, BIT_SW
    if kind == '0':
        body = (f'<rect x="{-w}" y="{-h}" width="{2*w}" height="{2*h}" rx="{w}" '
                f'fill="none" stroke="#EAFEFF" stroke-width="{sw}"/>')
    else:
        body = (f'<path d="M {-16} {-h+11} L 0 {-h} L 0 {h}" fill="none" stroke="#EAFEFF" '
                f'stroke-width="{sw}" stroke-linecap="round" stroke-linejoin="round"/>'
                f'<path d="M {-18} {h} L 18 {h}" fill="none" stroke="#EAFEFF" '
                f'stroke-width="{sw}" stroke-linecap="round"/>')
    return f'  <g transform="translate({x:.1f} {y:.1f})" opacity="{opacity}">{body}</g>'


def cloud_body():
    """云体：底座 + 圆穹并集，铺同一 userSpaceOnUse 渐变 → 看不出接缝"""
    b = CLOUD_BASE
    parts = [f'    <rect x="{b["x0"]}" y="{b["y0"]}" width="{b["x1"]-b["x0"]}" '
             f'height="{b["y1"]-b["y0"]}" rx="{b["r"]}" fill="url(#cloud)"/>']
    for cx, cy, r in CLOUD_LOBES:
        parts.append(f'    <circle cx="{cx}" cy="{cy}" r="{r}" fill="url(#cloud)"/>')
    return "\n".join(parts)


def cloud_clip():
    b = CLOUD_BASE
    lobes = " ".join(f'<circle cx="{cx}" cy="{cy}" r="{r}"/>' for cx, cy, r in CLOUD_LOBES)
    return (f'<rect x="{b["x0"]}" y="{b["y0"]}" width="{b["x1"]-b["x0"]}" '
            f'height="{b["y1"]-b["y0"]}" rx="{b["r"]}"/>  {lobes}')


def cloud_edge():
    """云体轮廓光：把底座与各圆穹各外扩 CLOUD_EDGE px 先铺一层淡蓝白，
    再叠正常云体 —— 于是整朵云外缘留下一圈极淡的边光，任何尺寸下剪影都不会融进背景。"""
    b = CLOUD_BASE
    p = CLOUD_EDGE
    parts = [f'  <rect x="{b["x0"]-p}" y="{b["y0"]-p}" width="{(b["x1"]-b["x0"])+2*p}" '
             f'height="{(b["y1"]-b["y0"])+2*p}" rx="{b["r"]+p}" fill="#BBDCFF" fill-opacity="0.16"/>']
    for cx, cy, r in CLOUD_LOBES:
        parts.append(f'  <circle cx="{cx}" cy="{cy}" r="{r+p}" fill="#BBDCFF" fill-opacity="0.16"/>')
    return "\n".join(parts)


def mark():
    """标记本体：环 → 光带 → 数据位 → 云体（云最后画，于是"坐"在环上）"""
    b = CLOUD_BASE
    bits = "\n".join(
        bit_glyph(kind, x, y + BIT_DY, round(0.95 - i * 0.13, 2))
        for i, ((x, y), kind) in enumerate(zip(bits_on_bottom(BIT_X), BIT_SEQ)))

    ring = f"""  <ellipse cx="{RING_CX}" cy="{RING_CY}" rx="{RING_RX}" ry="{RING_RY}"
           transform="rotate({RING_ROT} {RING_CX} {RING_CY})"
           fill="none" stroke="#00E5FF" stroke-opacity="0.05" stroke-width="60"/>
  <ellipse cx="{RING_CX}" cy="{RING_CY}" rx="{RING_RX}" ry="{RING_RY}"
           transform="rotate({RING_ROT} {RING_CX} {RING_CY})"
           fill="none" stroke="#00E5FF" stroke-opacity="0.13" stroke-width="{RING_STROKE}"/>"""
    arc = (f'  <path d="{ring_arc_path(ARC_T0, ARC_T1)}" fill="none" stroke="#00E5FF" '
           f'stroke-opacity="0.10" stroke-width="54" stroke-linecap="round"/>\n'
           f'  <path d="{ring_arc_path(ARC_T0, ARC_T1)}" fill="none" stroke="url(#streak)" '
           f'stroke-width="{ARC_STROKE}" stroke-linecap="round"/>')
    cloud = (f'  <g>{chr(10)}{cloud_edge()}{chr(10)}  </g>\n'
             f'  <g>{chr(10)}{cloud_body()}{chr(10)}  </g>\n'
             f'  <g clip-path="url(#cloudClip)">\n'
             f'    <rect x="100" y="150" width="824" height="500" fill="url(#cloudHi)"/>\n'
             f'    <rect x="150" y="520" width="724" height="130" fill="url(#cloudRim)"/>\n'
             f'  </g>')
    return "\n".join([ring, arc, bits, cloud])


DEFS = f"""  <defs>
    <!-- 底面：上浅下深，模拟自上而下的环境光 -->
    <linearGradient id="bg" x1="0" y1="0" x2="0.15" y2="1">
      <stop offset="0" stop-color="#1C2C4A"/>
      <stop offset="0.52" stop-color="#121E35"/>
      <stop offset="1" stop-color="#0A101C"/>
    </linearGradient>
    <!-- 顶部柔光：让平面有材质感，而不是一块纯色 -->
    <radialGradient id="sheen" cx="0.5" cy="0.02" r="0.78">
      <stop offset="0" stop-color="#FFFFFF" stop-opacity="0.11"/>
      <stop offset="0.5" stop-color="#FFFFFF" stop-opacity="0.03"/>
      <stop offset="1" stop-color="#FFFFFF" stop-opacity="0"/>
    </radialGradient>
    <!-- 底部内阴影：制造厚度 -->
    <linearGradient id="floor" x1="0" y1="0" x2="0" y2="1">
      <stop offset="0.55" stop-color="#000000" stop-opacity="0"/>
      <stop offset="1" stop-color="#000000" stop-opacity="0.30"/>
    </linearGradient>
    <!-- 标记背后的环境光：云"照亮"了整块底面 -->
    <radialGradient id="ambient" cx="0.5" cy="0.5" r="0.5">
      <stop offset="0" stop-color="#00B8DC" stop-opacity="0.20"/>
      <stop offset="0.55" stop-color="#0090B4" stop-opacity="0.07"/>
      <stop offset="1" stop-color="#007296" stop-opacity="0"/>
    </radialGradient>
    <!-- 云体：上受光、下沉入暗部 -->
    <linearGradient id="cloud" gradientUnits="userSpaceOnUse" x1="300" y1="195" x2="700" y2="630">
      <stop offset="0" stop-color="#7096BE"/>
      <stop offset="0.42" stop-color="#40608A"/>
      <stop offset="1" stop-color="#25405F"/>
    </linearGradient>
    <linearGradient id="cloudHi" gradientUnits="userSpaceOnUse" x1="0" y1="200" x2="0" y2="470">
      <stop offset="0" stop-color="#FFFFFF" stop-opacity="0.30"/>
      <stop offset="1" stop-color="#FFFFFF" stop-opacity="0"/>
    </linearGradient>
    <!-- 云底青色反光：平台在线的那道边光 -->
    <linearGradient id="cloudRim" gradientUnits="userSpaceOnUse" x1="0" y1="560" x2="0" y2="626">
      <stop offset="0" stop-color="#7FEEFF" stop-opacity="0"/>
      <stop offset="0.7" stop-color="#8FF2FF" stop-opacity="0.32"/>
      <stop offset="1" stop-color="#DFFDFF" stop-opacity="0.62"/>
    </linearGradient>
    <!-- 光带：左尾隐没、右头最亮，读作"数据正在流过" -->
    <linearGradient id="streak" gradientUnits="userSpaceOnUse" x1="300" y1="716" x2="716" y2="726">
      <stop offset="0" stop-color="#00C8E8" stop-opacity="0.12"/>
      <stop offset="0.55" stop-color="#4FDFFF" stop-opacity="0.65"/>
      <stop offset="1" stop-color="#EAFEFF" stop-opacity="1"/>
    </linearGradient>
    <clipPath id="bodyClip"><path d="{squircle_path(BODY)}"/></clipPath>
    <clipPath id="cloudClip">{cloud_clip()}</clipPath>
  </defs>"""


def app_icon_svg():
    body = squircle_path(BODY)
    hairline = squircle_path(BODY - 22)     # 内描边（贴内缘一圈高光）
    return f"""<svg xmlns="http://www.w3.org/2000/svg" width="{CANVAS}" height="{CANVAS}" viewBox="0 0 {CANVAS} {CANVAS}">
{DEFS}
  <!-- 圆角方底 -->
  <path d="{body}" fill="url(#bg)"/>
  <g clip-path="url(#bodyClip)">
    <rect x="{INSET}" y="{INSET}" width="{BODY}" height="{BODY}" fill="url(#sheen)"/>
    <rect x="{INSET}" y="{INSET}" width="{BODY}" height="{BODY}" fill="url(#floor)"/>
    <circle cx="{CX}" cy="{CY}" r="430" fill="url(#ambient)"/>
  </g>
  <path d="{hairline}" fill="none" stroke="#FFFFFF" stroke-opacity="0.07" stroke-width="2"/>

{mark()}
</svg>
"""


def tray_template_svg():
    """菜单栏 / 托盘模板图标：纯黑 + alpha，由系统按明暗主题着色。

    只保留**云体剪影**：菜单栏图标只有 16~22pt，环绕环在这个尺度会缩成一根发丝、
    0/1 数据位会变成脏点 —— Apple 自家菜单栏图标也都是把 App 图标简化成单一剪影。
    整体放大到画布约 78% 高，保证栅格化成清晰像素。
    """
    b = CLOUD_BASE
    parts = [f'  <rect x="{b["x0"]}" y="{b["y0"]}" width="{b["x1"]-b["x0"]}" '
             f'height="{b["y1"]-b["y0"]}" rx="{b["r"]}" fill="#000000"/>']
    for cx, cy, r in CLOUD_LOBES:
        parts.append(f'  <circle cx="{cx}" cy="{cy}" r="{r}" fill="#000000"/>')
    body = "\n".join(parts)
    return f"""<svg xmlns="http://www.w3.org/2000/svg" width="{CANVAS}" height="{CANVAS}" viewBox="0 0 {CANVAS} {CANVAS}">
  <g transform="translate({CX} {CY}) scale(1.34) translate({-CX} {-CY})">
{body}
  </g>
</svg>
"""


def main():
    os.makedirs(ICONS, exist_ok=True)

    with open(os.path.join(ICONS, "app-icon.svg"), "w", encoding="utf-8") as f:
        f.write(app_icon_svg())
    with open(os.path.join(ICONS, "tray-template.svg"), "w", encoding="utf-8") as f:
        f.write(tray_template_svg())

    # iOS 图标不能带透明（App Store 会拒），用 bg_color 兜底成品牌深色底 —— 与源图边缘色
    # 同族，圆角外那一圈几乎看不出来。桌面三平台（macOS/Windows/Linux）用源图本身的圆角几何。
    manifest = {
        "default": "app-icon.svg",
        "bg_color": "#0B1120",
    }
    with open(os.path.join(ICONS, "icon-manifest.json"), "w", encoding="utf-8") as f:
        json.dump(manifest, f, indent=2)
        f.write("\n")

    print("wrote:", os.path.join(ICONS, "app-icon.svg"))
    print("wrote:", os.path.join(ICONS, "tray-template.svg"))
    print("wrote:", os.path.join(ICONS, "icon-manifest.json"))

    # --- 栅格化：交给 tauri icon（resvg 内核），不自己造轮子 ---
    cli = os.path.join(REPO, "node_modules", ".bin", "tauri")
    if not os.path.exists(cli):
        print("skip rasterize: node_modules/.bin/tauri not found (run pnpm install)", file=sys.stderr)
        return

    # 1) 全平台图标（默认输出到 tauri.conf.json 旁的 icons/，即本目录）
    run([cli, "icon", os.path.join(ICONS, "icon-manifest.json")])

    # 2) 托盘模板图：单独指定尺寸，名字对齐 main.rs 的 include_bytes
    with tempfile.TemporaryDirectory() as tmp:
        run([cli, "icon", os.path.join(ICONS, "tray-template.svg"), "-p", str(TRAY_PX), "-o", tmp])
        shutil.copyfile(os.path.join(tmp, f"{TRAY_PX}x{TRAY_PX}.png"),
                        os.path.join(ICONS, "tray-template.png"))
    print("wrote:", os.path.join(ICONS, "tray-template.png"))


if __name__ == "__main__":
    main()
