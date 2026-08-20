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

    /// 由实例的两条事实推出档位。
    ///
    /// 之所以做成纯函数而不是各处现判：设置页要显示「自动会给多少」，启动时要真的按那个数分配，
    /// 两处算出不同的档位就等于界面在说谎。事实由调用方各自取（启动那边手上已经有，
    /// 设置页那边要现查），判定只此一份。
    ///
    /// 受管整合包一律按大型整合算：Aurora 装的就是那一个包，98 个 Mod 落在 Modded 的 4G 上限里偏紧。
    pub fn resolve(is_managed_modpack: bool, has_mod_loader: bool) -> Self {
        if is_managed_modpack {
            MemoryTier::LargeModpack
        } else if has_mod_loader {
            MemoryTier::Modded
        } else {
            MemoryTier::Vanilla
        }
    }
}

/// 本机物理内存快照（MB）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct SystemMemory {
    /// 物理内存总量。
    pub total_mb: u32,
    /// 当前可用量。Windows 不区分 free 与 available，这里就是 `MEMORYSTATUSEX.ullAvailPhys`。
    pub available_mb: u32,
}

impl SystemMemory {
    /// 此刻被其它程序占用的量（MB）。
    pub fn used_by_others_mb(self) -> u32 {
        self.total_mb.saturating_sub(self.available_mb)
    }
}

/// 探测本机物理内存。
///
/// 只刷 RAM 一项：默认的 `System::new_all` 会连进程表、CPU、磁盘一起拉，那是几十毫秒级的开销，
/// 而这里只要两个数。sysinfo 的字节数用 u64，转 MB 时按饱和处理——真有超过 4PB 的机器也不该 panic。
pub fn probe_system_memory() -> SystemMemory {
    let system = sysinfo::System::new_with_specifics(
        sysinfo::RefreshKind::nothing().with_memory(sysinfo::MemoryRefreshKind::nothing().with_ram()),
    );
    let to_mb = |bytes: u64| u32::try_from(bytes / (1024 * 1024)).unwrap_or(u32::MAX);
    SystemMemory {
        total_mb: to_mb(system.total_memory()),
        available_mb: to_mb(system.available_memory()),
    }
}

/// 滑块最低刻度（MB）。`slider_to_mb(0)` 是 0，那是个跑不起来的值，不进阶梯。
pub const MIN_STOP_MB: u32 = 512;

/// 滑块刻度阶梯：下标即滑块位置，值是该位置对应的最大堆（MB）。
///
/// 为什么要把整张表交出去，而不是让界面自己实现一遍分段折线：[`slider_to_mb`] 的三个折点
/// （2G / 6G / 14G）是这套手感的全部内容，复制到前端就成了两份实现，改一边不改另一边不会有任何报错，
/// 只会让滑块的手感悄悄漂掉。表由这里生成，界面只负责渲染下标。
///
/// 上界按本机物理内存截断：给一台 8G 的机器拖出 64G 的行程，拖到哪都是错的。
pub fn slider_stops(total_mb: u32) -> Vec<u32> {
    // 物理内存比最低刻度还小的机器（虚拟机、容器）也得有一格可选，否则界面拿到空表没法渲染。
    let ceiling = total_mb.max(MIN_STOP_MB);
    let mut stops = Vec::new();
    let mut slider = 0u32;
    loop {
        let mb = slider_to_mb(slider);
        if mb > ceiling {
            break;
        }
        if mb >= MIN_STOP_MB {
            stops.push(mb);
        }
        slider += 1;
    }
    stops
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
    fn tier_resolution_prefers_managed_modpack() {
        // 受管整合包压过「有没有加载器」这条：装的就是那个包，档位不该因为探测口径不同而变。
        assert_eq!(MemoryTier::resolve(true, true), MemoryTier::LargeModpack);
        assert_eq!(MemoryTier::resolve(true, false), MemoryTier::LargeModpack);
        assert_eq!(MemoryTier::resolve(false, true), MemoryTier::Modded);
        assert_eq!(MemoryTier::resolve(false, false), MemoryTier::Vanilla);
    }

    #[test]
    fn slider_stops_start_at_floor_and_stop_at_physical_memory() {
        // 16G 机器：从 512 起步，最后一格不超过物理内存。
        let stops = slider_stops(16384);
        assert_eq!(stops.first().copied(), Some(MIN_STOP_MB));
        assert_eq!(stops.last().copied(), Some(16384));
        // 严格递增，且不含 slider_to_mb(0) 那个 0。
        assert!(stops.windows(2).all(|w| w[0] < w[1]));
        assert!(!stops.contains(&0));
        // 三段折线原样体现在表里：2G 之前每格 0.25G，之后每格 0.5G，6G 之后每格 1G。
        assert!(stops.contains(&1024));
        assert!(stops.contains(&2048));
        assert!(stops.contains(&2560));
        assert!(stops.contains(&6144));
        assert!(stops.contains(&7168));
    }

    #[test]
    fn slider_stops_truncate_to_small_machines_and_never_empty() {
        // 4G 机器拖不出 8G 的行程。
        let small = slider_stops(4096);
        assert_eq!(small.last().copied(), Some(4096));
        assert!(!small.iter().any(|&mb| mb > 4096));

        // 物理内存低于最低刻度（容器/瘦虚拟机）时仍须给出可渲染的一格。
        let tiny = slider_stops(256);
        assert_eq!(tiny, vec![MIN_STOP_MB]);
    }

    #[test]
    fn system_memory_reports_others_usage() {
        let snapshot = SystemMemory {
            total_mb: 32768,
            available_mb: 12288,
        };
        assert_eq!(snapshot.used_by_others_mb(), 20480);
        // 可用大于总量是不可能的，但真读到脏数据也不该 panic 成负数。
        let broken = SystemMemory {
            total_mb: 1024,
            available_mb: 4096,
        };
        assert_eq!(broken.used_by_others_mb(), 0);
    }

    #[test]
    fn probe_reports_a_plausible_machine() {
        let probed = probe_system_memory();
        // 不钉具体数值（换台机器就变），钉的是「真读到了东西」与内部自洽。
        assert!(probed.total_mb >= 1024, "总内存 {} MB 不像真机", probed.total_mb);
        assert!(probed.available_mb <= probed.total_mb);
        assert!(probed.used_by_others_mb() < probed.total_mb);
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
