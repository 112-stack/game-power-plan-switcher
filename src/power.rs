// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (c) 2026 NN6. See LICENSE.txt and NOTICE.txt.
//! The mutation boundary can be tested with a fake Windows power provider.
pub trait Power {
    fn active(&mut self) -> Result<String, String>;
    fn set(&mut self, guid: &str) -> Result<(), String>;
}
pub struct Windows;
impl Power for Windows {
    fn active(&mut self) -> Result<String, String> {
        crate::win::active()
    }
    fn set(&mut self, guid: &str) -> Result<(), String> {
        crate::win::set_plan(guid)
    }
}
pub fn select(api: &mut impl Power, target: &str) -> Result<bool, String> {
    let before = api.active()?;
    if before.eq_ignore_ascii_case(target) {
        return Ok(false);
    }
    api.set(target)?;
    if !api.active()?.eq_ignore_ascii_case(target) {
        return Err("Windows did not retain the requested power plan.".into());
    }
    Ok(true)
}
#[cfg(test)]
mod tests {
    use super::*;
    struct Fake {
        active: String,
        writes: usize,
        fail: bool,
        ignore: bool,
        read_fail: bool,
    }
    impl Power for Fake {
        fn active(&mut self) -> Result<String, String> {
            if self.read_fail {
                Err("read denied".into())
            } else {
                Ok(self.active.clone())
            }
        }
        fn set(&mut self, g: &str) -> Result<(), String> {
            self.writes += 1;
            if self.fail {
                return Err("write denied".into());
            }
            if !self.ignore {
                self.active = g.into()
            }
            Ok(())
        }
    }
    fn fake() -> Fake {
        Fake {
            active: "balanced".into(),
            writes: 0,
            fail: false,
            ignore: false,
            read_fail: false,
        }
    }
    #[test]
    fn no_redundant_writes() {
        let mut f = fake();
        assert!(!select(&mut f, "BALANCED").unwrap());
        assert_eq!(f.writes, 0)
    }
    #[test]
    fn verifies_switch() {
        let mut f = fake();
        assert!(select(&mut f, "gaming").unwrap());
        assert_eq!(f.writes, 1);
        assert!(!select(&mut f, "gaming").unwrap());
        assert_eq!(f.writes, 1)
    }
    #[test]
    fn read_failure_never_writes() {
        let mut f = fake();
        f.read_fail = true;
        assert!(select(&mut f, "gaming").is_err());
        assert_eq!(f.writes, 0)
    }
    #[test]
    fn write_failure_can_retry() {
        let mut f = fake();
        f.fail = true;
        assert!(select(&mut f, "gaming").is_err());
        f.fail = false;
        assert!(select(&mut f, "gaming").unwrap())
    }
    #[test]
    fn ignored_switch_is_error() {
        let mut f = fake();
        f.ignore = true;
        assert!(select(&mut f, "gaming").is_err())
    }
}
