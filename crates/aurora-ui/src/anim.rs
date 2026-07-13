//! 弹簧动画内核。
//!
//! 采用两参阻尼谐振子模型（质量固定为 1）：给定刚度 k 与阻尼 c，用半隐式欧拉
//! 逐帧积分。之所以用半隐式欧拉（先更新速度再更新位置）而非显式欧拉，是因为
//! 它对弹簧这类刚性系统在大 dt 下更稳定，不会像显式欧拉那样发散。
//!
//! 五枚预设由 duration/bounce 两参换算而来（见 docs/frontend-design.md 动效一节）：
//! ω=2π/duration，k=ω²，c=2(1−bounce)ω，ζ=1−bounce。

use std::f32::consts::PI;

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
// 本 playground 切片走全局实时调参，暂不引用这些常量；保留导出供后续正式界面按语义取用，
// 故逐个标注 allow(dead_code)，避免被当作废弃代码删除或触发 -D warnings。
/// 按压/悬停：ζ=1.00 临界阻尼，零过冲，手感干脆。
#[allow(dead_code)]
pub const TAP: Params = Params::new(815.6, 57.12);
/// 卡片落定/滑块跟随：ζ=0.90，最常用。
#[allow(dead_code)]
pub const SETTLE: Params = Params::new(584.0, 43.50);
/// 原位往返归位：ζ=0.80，过冲最明显。
#[allow(dead_code)]
pub const POP: Params = Params::new(385.5, 31.42);
/// 整页入场：ζ=0.88。
#[allow(dead_code)]
pub const SOFT: Params = Params::new(341.5, 32.53);
/// 图标↔菜单形变：ζ=0.86。
#[allow(dead_code)]
pub const MORPH: Params = Params::new(322.3, 30.88);

// ---- Aurora 默认手感预设 ----
//
// 用户实测选定的三档手感（偏华丽）。以函数而非 const 暴露，避免手算 k/c 常量随 duration/bounce
// 调整而漂移失真：换算逻辑只此一处（[`params_from`]），预设即其命名调用点。

/// Aurora 默认弹簧（bounce 0.35 / duration 0.30s，ζ=0.65）。大多数过渡的默认手感。
pub fn aurora() -> Params {
    params_from(0.35, 0.30)
}

/// 按压/悬停更脆的手感（bounce 0.10 / duration 0.22s，ζ=0.90）。用于按钮按下、图标点按。
/// 供页面 agent 给按压形变取用（外壳当前未直接使用）。
#[allow(dead_code)]
pub fn aurora_press() -> Params {
    params_from(0.10, 0.22)
}

/// 整页入场略软的手感（bounce 0.30 / duration 0.34s，ζ=0.70）。用于页面切换、卡片首次落定。
pub fn aurora_enter() -> Params {
    params_from(0.30, 0.34)
}

/// 导航栏宽度这类布局量的脆弹簧（bounce 0.0 / duration 0.24s，ζ=1.00 临界阻尼）。宽度是布局量，
/// 过冲会让侧边栏「抽一下」并让 hover 命中边界来回扫过光标造成抖动；故取零过冲、脆而快的手感。
pub fn aurora_rail() -> Params {
    params_from(0.0, 0.24)
}

/// 两参手感模型：由弹跳强度 `bounce` 与到位时长 `duration`（秒，质量恒为 1）换算刚度/阻尼。
/// ω=2π/duration，k=ω²，c=2(1−bounce)ω。bounce 即 1−ζ，故阻尼比 ζ=1−bounce：
/// bounce=0 得临界阻尼零过冲，bounce 越大越欠阻尼、过冲越明显。
pub fn params_from(bounce: f32, duration: f32) -> Params {
    let omega = 2.0 * PI / duration;
    Params::new(omega * omega, 2.0 * (1.0 - bounce) * omega)
}

/// 标准二阶欠阻尼系统的阶跃超调百分比 Mp。ζ=1−bounce：ζ≥1（bounce≤0）时无过冲返回 0，
/// 否则 Mp = exp(−π·ζ/√(1−ζ²))·100。用于把「bounce 数值」翻译成用户能感知的过冲幅度。
/// 供调参界面/页面 agent 展示手感取用（外壳当前未直接使用）。
#[allow(dead_code)]
pub fn overshoot_percent(bounce: f32) -> f32 {
    let zeta = 1.0 - bounce;
    if zeta >= 1.0 {
        0.0
    } else {
        (-PI * zeta / (1.0 - zeta * zeta).sqrt()).exp() * 100.0
    }
}

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

/// 便捷的动画值封装：内含一枚 [`Spring`]，把「设目标 / 逐帧推进 / 取当前值 / 判静止」收敛成一个
/// 最小接口，供组件与页面持有。多维动画（x/y/缩放）持多枚 [`Animated`] 即可。
///
/// 与直接用 [`Spring`] 相比，[`Animated`] 只暴露语义方法（不暴露 k/c/velocity 字段），页面无需关心
/// 积分细节；改手感用 [`set_params`](Self::set_params)（保留 current/velocity，可中断切换）。
#[derive(Debug, Clone, Copy)]
pub struct Animated {
    spring: Spring,
}

impl Animated {
    /// 以初值与手感创建静止动画值（current==target，速度 0）。
    pub fn new(value: f32, params: Params) -> Self {
        Self {
            spring: Spring::new(value, params),
        }
    }

    /// 设定新目标，保留当前速度（可中断、有惯性）。
    pub fn set(&mut self, target: f32) {
        self.spring.set_target(target);
    }

    /// 推进一帧（dt 秒，内部钳制到 [`MAX_DT`]）。
    pub fn step(&mut self, dt: f32) {
        self.spring.step(dt);
    }

    /// 当前值（用于插值渲染，如位移/缩放/透明度）。
    pub fn value(&self) -> f32 {
        self.spring.current
    }

    /// 当前目标值。供页面 agent 判断动画朝向（外壳当前未直接使用）。
    #[allow(dead_code)]
    pub fn target(&self) -> f32 {
        self.spring.target
    }

    /// 是否已收敛静止（用于决定是否继续挂帧订阅）。
    pub fn settled(&self) -> bool {
        self.spring.settled()
    }

    /// 换手感：只改 k/c，保留 current/velocity/target。进行中的动画会当帧平滑改变手感、不跳变。
    /// 供页面 agent 运行期切换手感取用（外壳当前未直接使用）。
    #[allow(dead_code)]
    pub fn set_params(&mut self, params: Params) {
        self.spring.k = params.k;
        self.spring.c = params.c;
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

    #[test]
    fn params_from_matches_two_param_model() {
        let p = params_from(0.0, 0.30);
        let omega = 2.0 * PI / 0.30;
        // k=ω²，bounce=0 时 c=2ω（临界阻尼）。
        assert!((p.k - omega * omega).abs() < 1e-3, "k 换算不符 k={}", p.k);
        assert!((p.c - 2.0 * omega).abs() < 1e-3, "c 换算不符 c={}", p.c);
    }

    #[test]
    fn overshoot_is_zero_at_critical_damping() {
        // bounce=0 → ζ=1 → 临界阻尼，无过冲。
        assert_eq!(overshoot_percent(0.0), 0.0);
    }

    #[test]
    fn overshoot_grows_with_bounce() {
        let low = overshoot_percent(0.2);
        let high = overshoot_percent(0.6);
        assert!(low > 0.0, "欠阻尼应有正过冲 low={low}");
        assert!(high > low, "bounce 越大过冲越大 low={low} high={high}");
        // 已知值校核：bounce=0.2 → ζ=0.8 → Mp≈1.52%。
        assert!((low - 1.516).abs() < 0.05, "bounce=0.2 过冲应≈1.5% 实得 {low}");
    }

    #[test]
    fn aurora_preset_matches_selected_feel() {
        // 用户实测选定：bounce 0.35 / duration 0.30。预设必须等于该两参换算，不得漂移。
        let p = aurora();
        let expect = params_from(0.35, 0.30);
        assert_eq!(p, expect);
        // ζ=1−bounce=0.65 欠阻尼：驱动真实弹簧必然过冲。
        let mut s = Spring::new(0.0, p);
        s.set_target(100.0);
        let peak = run_to_peak(&mut s, 1.0 / 240.0, 2000);
        assert!(peak > 100.5, "Aurora 默认应有过冲 peak={peak}");
    }

    #[test]
    fn aurora_press_is_crisper_than_default() {
        // 按压手感 ζ=0.90 明显比默认 ζ=0.65 更接近临界阻尼，过冲更小。
        let mut press = Spring::new(0.0, aurora_press());
        press.set_target(100.0);
        let press_peak = run_to_peak(&mut press, 1.0 / 240.0, 2000);

        let mut default = Spring::new(0.0, aurora());
        default.set_target(100.0);
        let default_peak = run_to_peak(&mut default, 1.0 / 240.0, 2000);

        assert!(
            press_peak < default_peak,
            "按压应比默认更脆 press={press_peak} default={default_peak}"
        );
    }

    #[test]
    fn aurora_rail_settles_without_overshoot() {
        // 导航栏脆弹簧 ζ=1.00 临界阻尼：宽度弹到目标不得越过（避免侧边栏「抽一下」与 hover 边界抖动）。
        let mut s = Spring::new(0.0, aurora_rail());
        s.set_target(100.0);
        let peak = run_to_peak(&mut s, 1.0 / 240.0, 2000);
        assert!(peak <= 100.5, "导航栏脆弹簧不应过冲 peak={peak}");
        assert!(s.settled());
        assert_eq!(s.current, 100.0);
        assert_eq!(s.velocity, 0.0);
    }

    #[test]
    fn animated_tracks_target_and_settles_exactly() {
        let mut a = Animated::new(0.0, aurora());
        a.set(64.0);
        assert_eq!(a.target(), 64.0);
        for _ in 0..3000 {
            a.step(1.0 / 120.0);
            if a.settled() {
                break;
            }
        }
        assert!(a.settled());
        // 收敛后严格吸附到目标。
        assert_eq!(a.value(), 64.0);
    }

    #[test]
    fn animated_set_params_preserves_motion() {
        let mut a = Animated::new(0.0, aurora());
        a.set(100.0);
        a.step(1.0 / 120.0);
        let mid = a.value();
        assert!(mid > 0.0, "第一帧后应已位移");
        // 换手感不应把值或目标清零（可中断切换）。
        a.set_params(aurora_press());
        assert_eq!(a.value(), mid);
        assert_eq!(a.target(), 100.0);
    }

    #[test]
    fn params_from_bounce_drives_spring_overshoot() {
        // 用换算参数驱动真实弹簧：bounce=0 临界阻尼不越过目标；bounce=0.5 欠阻尼必过冲。
        let mut crit = Spring::new(0.0, params_from(0.0, 0.30));
        crit.set_target(100.0);
        let crit_peak = run_to_peak(&mut crit, 1.0 / 240.0, 2000);
        assert!(crit_peak <= 100.5, "bounce=0 不应过冲 peak={crit_peak}");

        let mut bouncy = Spring::new(0.0, params_from(0.5, 0.30));
        bouncy.set_target(100.0);
        let bouncy_peak = run_to_peak(&mut bouncy, 1.0 / 240.0, 2000);
        assert!(bouncy_peak > 101.0, "bounce=0.5 应过冲 peak={bouncy_peak}");
    }
}
