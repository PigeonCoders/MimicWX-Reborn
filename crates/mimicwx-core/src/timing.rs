//! 时间工具

/// 毫秒转 Duration 的简写
pub fn ms(n: u64) -> std::time::Duration {
    std::time::Duration::from_millis(n)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_ms() {
        assert_eq!(ms(0), std::time::Duration::from_millis(0));
        assert_eq!(ms(500), std::time::Duration::from_millis(500));
        assert_eq!(ms(1000), std::time::Duration::from_secs(1));
    }
}
