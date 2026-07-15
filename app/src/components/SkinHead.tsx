// 皮肤头像：照 PCL2 的 MySkin 复刻——纯 2D 双层，脸层 + 放大的帽层居中叠，nearest-neighbor 硬像素。
// PCL2 源码事实(Meloong-Git/PCL, MySkin.xaml/.vb)：脸层取皮肤 (8,8) 8x8、帽层取 (40,8) 8x8；
// 帽层画得比脸层大(原版 56:48≈1.167)并居中，四周外扩形成"那一圈"，靠的是 2.5D 微外扩而非真 3D；
// 缩放全程 NearestNeighbor 保持硬边像素(图床超采样平滑正是发糊的根因)。帽层仅在皮肤含透明或非纯色时画。
// 从原始皮肤 PNG 本地渲染，源服务抖动也只影响取图不影响渲染；取图失败回落名字首字母墨块，绝不留空。

import { useEffect, useRef, useState } from "react";

// 帽层占盒子比例(留一点白，PCL2 观感)。脸层由帽层反推，锁定 PCL2 的"帽>脸"放大比 56/48。
const HAT_RATIO = 0.94;
const FACE_RATIO = (HAT_RATIO * 48) / 56;

// 名字首个码位(Array.from 正确切割 CJK / 代理对)；空名兜底问号仅为字形占位。
function initialOf(name: string): string {
  return Array.from(name.trim())[0]?.toUpperCase() ?? "?";
}

// 帽层是否该画：全不透明且纯色 = 垃圾实心帽(放大只会糊一片盖住脸)，跳过；有透明或非纯色才画。照 PCL2 的判断。
function hatIsMeaningful(data: Uint8ClampedArray): boolean {
  const r0 = data[0];
  const g0 = data[1];
  const b0 = data[2];
  let allOpaque = true;
  let uniform = true;
  for (let i = 0; i < data.length; i += 4) {
    if (data[i + 3] < 255) {
      allOpaque = false;
      break;
    }
    if (data[i] !== r0 || data[i + 1] !== g0 || data[i + 2] !== b0) uniform = false;
  }
  return !allOpaque || !uniform;
}

export function SkinHead({ uuid, name, size }: { uuid: string; name: string; size: number }) {
  const canvasRef = useRef<HTMLCanvasElement>(null);
  const [failed, setFailed] = useState(false);

  useEffect(() => {
    setFailed(false);
    let cancelled = false;
    const img = new Image();
    // mc-heads 皮肤 ACAO:*，跨域取图不污染 canvas，可 getImageData 做帽层检测。
    img.crossOrigin = "anonymous";
    img.onload = () => {
      if (cancelled) return;
      const canvas = canvasRef.current;
      if (!canvas) return;
      const ctx = canvas.getContext("2d");
      if (!ctx) return;

      const dpr = window.devicePixelRatio || 1;
      canvas.width = Math.round(size * dpr);
      canvas.height = Math.round(size * dpr);
      ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
      ctx.imageSmoothingEnabled = false;
      ctx.clearRect(0, 0, size, size);

      const scale = img.naturalWidth / 64; // HD 皮肤(128/256)等比放大裁剪坐标
      const reg = 8 * scale; // head 正面区域边长(皮肤像素)

      // 脸层：皮肤 (8,8) 8x8 -> 居中方块。
      const faceD = size * FACE_RATIO;
      const faceOff = (size - faceD) / 2;
      ctx.drawImage(img, 8 * scale, 8 * scale, reg, reg, faceOff, faceOff, faceD, faceD);

      // 帽层：皮肤 (40,8) 8x8，先在离屏画布探测有效性(未污染 canvas 才能读像素)。
      let drawHat = true;
      const off = document.createElement("canvas");
      off.width = reg;
      off.height = reg;
      const octx = off.getContext("2d", { willReadFrequently: true });
      if (octx) {
        octx.imageSmoothingEnabled = false;
        octx.drawImage(img, 40 * scale, 8 * scale, reg, reg, 0, 0, reg, reg);
        drawHat = hatIsMeaningful(octx.getImageData(0, 0, reg, reg).data);
      }
      if (drawHat) {
        const hatD = size * HAT_RATIO;
        const hatOff = (size - hatD) / 2;
        ctx.drawImage(img, 40 * scale, 8 * scale, reg, reg, hatOff, hatOff, hatD, hatD);
      }
    };
    img.onerror = () => {
      if (!cancelled) setFailed(true);
    };
    img.src = `https://mc-heads.net/skin/${encodeURIComponent(uuid)}`;
    return () => {
      cancelled = true;
    };
  }, [uuid, size]);

  if (failed) {
    return (
      <span
        style={{ width: size, height: size }}
        className="grid shrink-0 place-items-center rounded-[3px] bg-ink font-extrabold text-paper-on"
        aria-label={name}
      >
        {initialOf(name)}
      </span>
    );
  }
  return (
    <canvas
      ref={canvasRef}
      width={size}
      height={size}
      style={{ width: size, height: size }}
      role="img"
      aria-label={`${name} 的皮肤头像`}
      className="shrink-0 [image-rendering:pixelated]"
    />
  );
}
