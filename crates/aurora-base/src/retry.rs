//! 指数退避重试包装。
//!
//! 网络/下载天然会遇到瞬时故障（超时、连接重置、镜像抽风）。本模块提供一个通用的
//! [`retry_async`]：仅当错误 [`RetryableError::is_retryable`] 为真且还有剩余次数时才退避重试，
//! 4xx/配置类错误立即上抛，不做无谓等待。
//!
//! 退避带 full-jitter，缓解成百上千个 asset 文件同时失败、同时重试造成的镜像雪崩。

use std::future::Future;
use std::time::Duration;

/// 标记某类错误是否值得重试。上游错误枚举实现它后即可直接喂给 [`retry_async`]。
pub trait RetryableError {
    /// 该错误是否属于「再试一次可能就好」的瞬时故障。
    fn is_retryable(&self) -> bool;
}

/// 退避策略。
#[derive(Debug, Clone)]
pub struct RetryPolicy {
    /// 最大尝试次数（含首次），至少为 1。
    pub max_attempts: u32,
    /// 首次重试前的基准等待。
    pub initial_delay: Duration,
    /// 单次等待上限（退避封顶）。
    pub max_delay: Duration,
    /// 每退避一轮的倍率。
    pub multiplier: f64,
    /// 是否施加 full-jitter（生产建议开，测试可关以求确定性）。
    pub jitter: bool,
}

impl Default for RetryPolicy {
    fn default() -> Self {
        Self {
            max_attempts: 4,
            initial_delay: Duration::from_millis(500),
            max_delay: Duration::from_secs(30),
            multiplier: 2.0,
            jitter: true,
        }
    }
}

impl RetryPolicy {
    /// 第 `attempt` 轮重试（从 0 计）的确定性退避基值，不含 jitter。
    ///
    /// 计算 `initial * multiplier^attempt` 并封顶到 `max_delay`。先在 f64 域内夹取再转回
    /// `Duration`，指数溢出会变成 `inf`，`min` 后仍被 `max_delay` 拉回，不会 panic。
    pub fn base_delay(&self, attempt: u32) -> Duration {
        let factor = self.multiplier.powi(attempt as i32);
        let millis = self.initial_delay.as_millis() as f64 * factor;
        let capped = millis.min(self.max_delay.as_millis() as f64).max(0.0);
        Duration::from_millis(capped as u64)
    }

    /// 实际等待：base_delay 之上按 full-jitter 在 `[0, base]` 内随机取值。
    fn actual_delay(&self, attempt: u32) -> Duration {
        let base = self.base_delay(attempt);
        if !self.jitter || base.is_zero() {
            return base;
        }
        let jittered = fastrand::u64(0..=base.as_millis() as u64);
        Duration::from_millis(jittered)
    }
}

/// 按 `policy` 对异步操作做指数退避重试。
///
/// `op` 每次被调用生成一个新 future（因此可携带重定向后的 URL、切换后的镜像源等状态）。
/// 成功即返回；失败时仅当错误可重试且尚有剩余次数才退避后再来一轮，否则原样上抛最后一次错误。
pub async fn retry_async<T, E, Op, Fut>(
    policy: &RetryPolicy,
    mut op: Op,
) -> std::result::Result<T, E>
where
    E: RetryableError,
    Op: FnMut() -> Fut,
    Fut: Future<Output = std::result::Result<T, E>>,
{
    let max = policy.max_attempts.max(1);
    let mut attempt: u32 = 0;
    loop {
        match op().await {
            Ok(value) => return Ok(value),
            Err(err) => {
                let is_last = attempt + 1 >= max;
                if is_last || !err.is_retryable() {
                    return Err(err);
                }
                let delay = policy.actual_delay(attempt);
                tracing::debug!(
                    attempt = attempt + 1,
                    max,
                    delay_ms = delay.as_millis() as u64,
                    "操作失败，退避后重试"
                );
                tokio::time::sleep(delay).await;
                attempt += 1;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};

    #[derive(Debug)]
    struct TestError {
        retryable: bool,
    }

    impl RetryableError for TestError {
        fn is_retryable(&self) -> bool {
            self.retryable
        }
    }

    fn fast_policy(max_attempts: u32) -> RetryPolicy {
        // 极小延迟 + 关 jitter：测试跑得快且行为确定。
        RetryPolicy {
            max_attempts,
            initial_delay: Duration::from_millis(1),
            max_delay: Duration::from_millis(4),
            multiplier: 2.0,
            jitter: false,
        }
    }

    #[test]
    fn base_delay_grows_then_caps() {
        let policy = RetryPolicy {
            max_attempts: 6,
            initial_delay: Duration::from_millis(100),
            max_delay: Duration::from_millis(1000),
            multiplier: 2.0,
            jitter: false,
        };
        assert_eq!(policy.base_delay(0), Duration::from_millis(100));
        assert_eq!(policy.base_delay(1), Duration::from_millis(200));
        assert_eq!(policy.base_delay(2), Duration::from_millis(400));
        assert_eq!(policy.base_delay(3), Duration::from_millis(800));
        // 1600 会被封顶到 1000。
        assert_eq!(policy.base_delay(4), Duration::from_millis(1000));
        assert_eq!(policy.base_delay(10), Duration::from_millis(1000));
    }

    #[test]
    fn actual_delay_with_jitter_stays_within_base() {
        let policy = RetryPolicy {
            max_attempts: 4,
            initial_delay: Duration::from_millis(200),
            max_delay: Duration::from_secs(30),
            multiplier: 2.0,
            jitter: true,
        };
        let base = policy.base_delay(3);
        for _ in 0..256 {
            assert!(policy.actual_delay(3) <= base);
        }
    }

    #[tokio::test]
    async fn succeeds_on_first_try() {
        let calls = Arc::new(AtomicUsize::new(0));
        let c = calls.clone();
        let result: Result<u32, TestError> = retry_async(&fast_policy(3), move || {
            let c = c.clone();
            async move {
                c.fetch_add(1, Ordering::SeqCst);
                Ok(7)
            }
        })
        .await;
        assert_eq!(result.unwrap(), 7);
        assert_eq!(calls.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn retries_until_success() {
        let calls = Arc::new(AtomicUsize::new(0));
        let c = calls.clone();
        let result: Result<u32, TestError> = retry_async(&fast_policy(5), move || {
            let c = c.clone();
            async move {
                let n = c.fetch_add(1, Ordering::SeqCst) + 1;
                if n < 3 {
                    Err(TestError { retryable: true })
                } else {
                    Ok(42)
                }
            }
        })
        .await;
        assert_eq!(result.unwrap(), 42);
        assert_eq!(calls.load(Ordering::SeqCst), 3);
    }

    #[tokio::test]
    async fn exhausts_attempts_then_returns_last_error() {
        let calls = Arc::new(AtomicUsize::new(0));
        let c = calls.clone();
        let result: Result<u32, TestError> = retry_async(&fast_policy(3), move || {
            let c = c.clone();
            async move {
                c.fetch_add(1, Ordering::SeqCst);
                Err(TestError { retryable: true })
            }
        })
        .await;
        assert!(result.is_err());
        // 恰好尝试 max_attempts 次，不多不少。
        assert_eq!(calls.load(Ordering::SeqCst), 3);
    }

    #[tokio::test]
    async fn non_retryable_short_circuits() {
        let calls = Arc::new(AtomicUsize::new(0));
        let c = calls.clone();
        let result: Result<u32, TestError> = retry_async(&fast_policy(5), move || {
            let c = c.clone();
            async move {
                c.fetch_add(1, Ordering::SeqCst);
                Err(TestError { retryable: false })
            }
        })
        .await;
        assert!(result.is_err());
        // 不可重试错误应只尝试一次。
        assert_eq!(calls.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn zero_max_attempts_is_treated_as_one() {
        let calls = Arc::new(AtomicUsize::new(0));
        let c = calls.clone();
        let result: Result<u32, TestError> = retry_async(&fast_policy(0), move || {
            let c = c.clone();
            async move {
                c.fetch_add(1, Ordering::SeqCst);
                Err(TestError { retryable: true })
            }
        })
        .await;
        assert!(result.is_err());
        assert_eq!(calls.load(Ordering::SeqCst), 1);
    }
}
