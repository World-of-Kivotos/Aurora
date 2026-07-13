//! 内存配置：自动分配、手动滑块换算，产出 `-Xmx` / `-Xms`。
//!
//! 自动分配按「版本档位」（原版 / 装 Mod / 大型整合）设不同的上限与下限，并按可用物理内存做分段递减
//! 供给：可用内存越多，愿意划给游戏的比例越低（给系统与其它进程留头绪），最后按档位夹逼并对齐到 128MB。
//! PCL 未开源其真实曲线，这里用一条可解释、可测的分段折线代替（数值都在下方常量里，便于调参）。

use serde::{Deserialize, Serialize};

/// `-Xmx` 对齐步长（MB）。分配结果向下取整到该步长，得到整洁的整数值。
const ALIGN_MB: u32 = 128;

/// 版本内存档位：越靠后，默认上限越高。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MemoryTier {
    /// 原版 / 无加载器：轻量。
    Vanilla,
    /// 安装了 Mod 加载器（Fabric/Forge/…）或 OptiFine：中量。
    Modded,
    /// 大型整合包：重量。
    LargeModpack,
}

impl MemoryTier {
    /// 该档位的最低分配（MB）：即使可用内存很少，也至少请求这么多（宁可 OOM 也别给一个跑不起来的值）。
    pub fn min_mb(self) -> u32 {
        match self {
            MemoryTier::Vanilla => 512,
            MemoryTier::Modded => 1024,
            MemoryTier::LargeModpack => 2048,
        }
    }

    /// 该档位的分配上限（MB）：再多的物理内存也不会超过它（原版给 16G 纯属浪费）。
    pub fn cap_mb(self) -> u32 {
        match self {
            MemoryTier::Vanilla => 2048,
            MemoryTier::Modded => 4096,
            MemoryTier::LargeModpack => 8192,
        }
    }
}

/// 一次内存配置：最大堆（必给）+ 可选最小堆。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct MemoryConfig {
    /// 最大堆 `-Xmx`（MB）。
    pub max_mb: u32,
    /// 最小堆 `-Xms`（MB）。`None` 表示不显式设置。
    pub min_mb: Option<u32>,
}

impl MemoryConfig {
    /// 固定最大堆，不设最小堆。
    pub fn fixed(max_mb: u32) -> Self {
        Self { max_mb, min_mb: None }
    }

    /// 链式设置最小堆。
    pub fn with_min(mut self, min_mb: u32) -> Self {
        self.min_mb = Some(min_mb);
        self
    }

    /// 按可用物理内存与版本档位自动分配最大堆。
    pub fn automatic(free_mb: u32, tier: MemoryTier) -> Self {
        Self::fixed(auto_allocate(free_mb, tier))
    }

    /// 产出内存相关的 JVM 参数。设了最小堆则 `-Xms` 在前、`-Xmx` 在后。
    pub fn jvm_args(&self) -> Vec<String> {
        let mut args = Vec::new();
        if let Some(min) = self.min_mb {
            args.push(format!("-Xms{min}m"));
        }
        args.push(format!("-Xmx{}m", self.max_mb));
        args
    }
}

/// 按可用物理内存与档位算出建议的最大堆（MB）。
///
/// 分段折线：把可用内存切成 [0,4G) / [4G,8G) / [8G,16G) / [16G,∞) 四段，分别按 50% / 40% / 25% / 10%
/// 的比例累加成「愿意划给游戏的量」，再向下对齐到 [`ALIGN_MB`]、按档位下限/上限夹逼。比例随内存增大递减，
/// 是为了在大内存机器上不把内存一股脑全塞给游戏。
pub fn auto_allocate(free_mb: u32, tier: MemoryTier) -> u32 {
    let free = f64::from(free_mb);
    let seg = |from: f64, width: f64, ratio: f64| (free - from).clamp(0.0, width) * ratio;
    let raw = seg(0.0, 4096.0, 0.50)
        + seg(4096.0, 4096.0, 0.40)
        + seg(8192.0, 8192.0, 0.25)
        + seg(16384.0, f64::INFINITY, 0.10);

    let aligned = (raw as u32) / ALIGN_MB * ALIGN_MB;
    aligned.clamp(tier.min_mb(), tier.cap_mb())
}

/// 手动内存滑块整数值 -> 实际分配 MB。分段线性，各段斜率不同：低内存区细粒度、高内存区粗粒度，
/// 让滑块在常用区间（2~8G）更好调。滑块 0 对应 0（不建议，但保留），随刻度递增。
pub fn slider_to_mb(slider: u32) -> u32 {
    match slider {
        0..=8 => slider * 256,                    // 0 .. 2048，斜率 256（每格 0.25G）
        9..=16 => 2048 + (slider - 8) * 512,      // 2560 .. 6144，斜率 512（每格 0.5G）
        17..=24 => 6144 + (slider - 16) * 1024,   // 7168 .. 14336，斜率 1024（每格 1G）
        _ => 14336 + (slider - 24) * 2048,        // 斜率 2048（每格 2G）
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn jvm_args_order_and_content() {
        assert_eq!(MemoryConfig::fixed(2048).jvm_args(), vec!["-Xmx2048m"]);
        assert_eq!(
            MemoryConfig::fixed(4096).with_min(1024).jvm_args(),
            vec!["-Xms1024m", "-Xmx4096m"]
        );
    }

    #[test]
    fn auto_allocate_pins_exact_values() {
        // 8G 可用：raw = 4096*0.5 + 4096*0.4 = 3686.4 -> 对齐 3584。
        assert_eq!(auto_allocate(8192, MemoryTier::Vanilla), 2048); // 夹到原版上限
        assert_eq!(auto_allocate(8192, MemoryTier::Modded), 3584);
        assert_eq!(auto_allocate(8192, MemoryTier::LargeModpack), 3584);

        // 16G 可用：raw = 2048 + 1638.4 + 2048 = 5734.4 -> 对齐 5632。
        assert_eq!(auto_allocate(16384, MemoryTier::Vanilla), 2048);
        assert_eq!(auto_allocate(16384, MemoryTier::Modded), 4096); // 夹到装 Mod 上限
        assert_eq!(auto_allocate(16384, MemoryTier::LargeModpack), 5632);

        // 32G 可用：raw = 2048 + 1638.4 + 2048 + 1638.4 = 7372.8 -> 对齐 7296。
        assert_eq!(auto_allocate(32768, MemoryTier::LargeModpack), 7296);
    }

    #[test]
    fn auto_allocate_respects_tier_floor_on_low_memory() {
        // 2G 可用：raw = 2048*0.5 = 1024 -> 对齐 1024。
        assert_eq!(auto_allocate(2048, MemoryTier::Vanilla), 1024); // 在 [512,2048] 内
        assert_eq!(auto_allocate(2048, MemoryTier::Modded), 1024); // 恰为下限
        // 大型整合下限 2048 高于算得的 1024，抬到下限。
        assert_eq!(auto_allocate(2048, MemoryTier::LargeModpack), 2048);
    }

    #[test]
    fn slider_segments_have_distinct_slopes() {
        assert_eq!(slider_to_mb(0), 0);
        assert_eq!(slider_to_mb(8), 2048);
        assert_eq!(slider_to_mb(9), 2560); // 进入 512 斜率段
        assert_eq!(slider_to_mb(16), 6144);
        assert_eq!(slider_to_mb(17), 7168); // 进入 1024 斜率段
        assert_eq!(slider_to_mb(24), 14336);
        assert_eq!(slider_to_mb(25), 16384); // 进入 2048 斜率段
    }
}
