//! 弹簧动画内核。
//!
//! 采用两参阻尼谐振子模型（质量固定为 1）：给定刚度 k 与阻尼 c，用半隐式欧拉
//! 逐帧积分。之所以用半隐式欧拉（先更新速度再更新位置）而非显式欧拉，是因为
//! 它对弹簧这类刚性系统在大 dt 下更稳定，不会像显式欧拉那样发散。
//!
//! 五枚预设由 duration/bounce 两参换算而来（见 docs/frontend-design.md 动效一节）：
//! ω=2π/duration，k=ω²，c=2(1−bounce)ω，ζ=1−bounce。

/// 单帧最大 dt（秒）。掉帧或后台切回时相邻时间戳可能相差很大，钳制到 1/30s
/// 避免一步积分把系统推飞。
pub const MAX_DT: f32 = 1.0 / 30.0;

/// 收敛阈值：位移与速度绝对值均低于此值即视为静止并吸附目标。
const REST_EPSILON: f32 = 0.01;

/// 弹簧参数：刚度 k 与阻尼 c。质量恒为 1，不单列。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Params {
    pub k: f32,
    pub c: f32,
}

impl Params {
    pub const fn new(k: f32, c: f32) -> Self {
        Self { k, c }
    }
}

// 五枚预设常量。注释里的 ζ 为阻尼比，用于快速判断过冲程度。
/// 按压/悬停：ζ=1.00 临界阻尼，零过冲，手感干脆。
pub const TAP: Params = Params::new(815.6, 57.12);
/// 卡片落定/滑块跟随：ζ=0.90，最常用。
pub const SETTLE: Params = Params::new(584.0, 43.50);
/// 原位往返归位：ζ=0.80，过冲最明显。
pub const POP: Params = Params::new(385.5, 31.42);
/// 整页入场：ζ=0.88。
pub const SOFT: Params = Params::new(341.5, 32.53);
/// 图标↔菜单形变：ζ=0.86。
pub const MORPH: Params = Params::new(322.3, 30.88);

/// 一维弹簧状态。多维动画（位置 x/y、缩放）用多枚 [`Spring`] 组合即可。
#[derive(Debug, Clone, Copy)]
pub struct Spring {
    pub current: f32,
    pub velocity: f32,
    pub target: f32,
    pub k: f32,
    pub c: f32,
    pub mass: f32,
}

impl Spring {
    /// 以给定初值创建静止弹簧（current==target，速度为 0）。
    pub fn new(value: f32, params: Params) -> Self {
        Self {
            current: value,
            velocity: 0.0,
            target: value,
            k: params.k,
            c: params.c,
            mass: 1.0,
        }
    }

    /// 只改目标，不清零速度。这是「可中断、有惯性」的关键：动画进行中再次改向
    /// 会带着当前速度平滑转向，而不是从零重新弹。
    pub fn set_target(&mut self, target: f32) {
        self.target = target;
    }

    /// 半隐式欧拉积分一帧。dt 会被钳制到 [`MAX_DT`]。
    ///
    /// 收敛后（[`settled`](Self::settled)）吸附到目标并把速度归零，避免在阈值附近
    /// 无限抖动，也让 [`settled`](Self::settled) 之后严格等于目标值。
    pub fn step(&mut self, dt: f32) {
        let dt = dt.clamp(0.0, MAX_DT);

        // F = -k·位移 - c·速度；a = F/m。先更新速度，再用新速度更新位置。
        let force = -self.k * (self.current - self.target) - self.c * self.velocity;
        let acceleration = force / self.mass;
        self.velocity += acceleration * dt;
        self.current += self.velocity * dt;

        if self.settled() {
            self.current = self.target;
            self.velocity = 0.0;
        }
    }

    /// 收敛判定：距目标位移与速度的绝对值均在阈值内。
    pub fn settled(&self) -> bool {
        (self.target - self.current).abs() < REST_EPSILON && self.velocity.abs() < REST_EPSILON
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 推进 `steps` 帧并返回过程中 current 的最大值（用于观测过冲峰值）。
    fn run_to_peak(spring: &mut Spring, dt: f32, steps: usize) -> f32 {
        let mut peak = spring.current;
        for _ in 0..steps {
            spring.step(dt);
            if spring.current > peak {
                peak = spring.current;
            }
        }
        peak
    }

    #[test]
    fn tap_is_critically_damped_without_overshoot() {
        let mut s = Spring::new(0.0, TAP);
        s.set_target(100.0);
        let peak = run_to_peak(&mut s, 1.0 / 240.0, 1200);
        // ζ=1：临界阻尼，不得越过目标（留极小数值误差余量）。
        assert!(peak <= 100.5, "tap 不应过冲，peak={peak}");
        assert!(s.settled());
        assert_eq!(s.current, 100.0);
        assert_eq!(s.velocity, 0.0);
    }

    #[test]
    fn pop_overshoots_the_target() {
        let mut s = Spring::new(0.0, POP);
        s.set_target(100.0);
        let peak = run_to_peak(&mut s, 1.0 / 240.0, 1200);
        // ζ=0.8：欠阻尼，必然越过目标后再回落。
        assert!(peak > 100.5, "pop 应过冲，peak={peak}");
        assert!(s.settled());
        assert_eq!(s.current, 100.0);
    }

    #[test]
    fn pop_overshoots_more_than_tap() {
        let mut tap = Spring::new(0.0, TAP);
        tap.set_target(100.0);
        let tap_peak = run_to_peak(&mut tap, 1.0 / 240.0, 1200);

        let mut pop = Spring::new(0.0, POP);
        pop.set_target(100.0);
        let pop_peak = run_to_peak(&mut pop, 1.0 / 240.0, 1200);

        // 手感差异的量化底线：pop 峰值显著高于 tap。
        assert!(
            pop_peak > tap_peak + 0.5,
            "过冲对比不明显 tap={tap_peak} pop={pop_peak}"
        );
    }

    #[test]
    fn settles_and_snaps_exactly_to_target() {
        let mut s = Spring::new(10.0, SETTLE);
        s.set_target(-40.0);
        for _ in 0..3000 {
            s.step(1.0 / 120.0);
            if s.settled() {
                break;
            }
        }
        assert!(s.settled());
        assert_eq!(s.current, -40.0);
        assert_eq!(s.velocity, 0.0);
    }

    #[test]
    fn dt_is_clamped_to_avoid_explosion() {
        let mut s = Spring::new(0.0, TAP);
        s.set_target(100.0);
        // 一步给一个远超一帧的 dt；若不钳制会把 current 推到天文数字。
        s.step(10.0);
        assert!(s.current.is_finite());
        assert!(s.current.abs() < 1000.0, "dt 未被钳制，current={}", s.current);
    }

    #[test]
    fn set_target_preserves_velocity_for_interruption() {
        let mut s = Spring::new(0.0, POP);
        s.set_target(100.0);
        s.step(1.0 / 120.0);
        let moving = s.velocity;
        assert!(moving > 0.0, "第一帧后应已获得正向速度");

        // 动画途中改向：速度必须原样保留（惯性 + 可中断）。
        s.set_target(-50.0);
        assert_eq!(s.velocity, moving);
    }
}
