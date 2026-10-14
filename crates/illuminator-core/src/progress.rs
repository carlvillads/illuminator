//! progress.rs
//!
//! Progress reports and cancelling for long work such as loading or measuring.
//!

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use crate::error::{Error, Result};

type ReportFn = dyn Fn(f64, &str) + Send + Sync;

#[derive(Clone)]
pub struct Progress {
    cancelled: Option<Arc<AtomicBool>>,
    report: Option<Arc<ReportFn>>,
    start: f64,
    extent: f64,
}

impl Default for Progress {
    /// reports nothing and is never cancelled
    fn default() -> Self {
        Self {
            cancelled: None,
            report: None,
            start: 0.0,
            extent: 1.0,
        }
    }
}

impl Progress {
    pub(crate) fn new(
        cancelled: Arc<AtomicBool>,
        report: impl Fn(f64, &str) + Send + Sync + 'static,
    ) -> Self {
        Self {
            cancelled: Some(cancelled),
            report: Some(Arc::new(report)),
            ..Self::default()
        }
    }

    pub(crate) fn part(&self, start: f64, extent: f64) -> Self {
        Self {
            start: self.start + self.extent * start,
            extent: self.extent * extent,
            ..self.clone()
        }
    }

    pub(crate) fn check(&self) -> Result<()> {
        match &self.cancelled {
            Some(flag) if flag.load(Ordering::Relaxed) => Err(Error::Cancelled),
            _ => Ok(()),
        }
    }

    pub(crate) fn report(&self, fraction: f64, stage: &str) -> Result<()> {
        self.check()?;
        if let Some(report) = &self.report {
            report(self.start + self.extent * fraction.clamp(0.0, 1.0), stage);
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use super::*;

    #[test]
    fn parts_map_onto_the_whole() {
        let values = Arc::new(Mutex::new(Vec::new()));
        let seen = Arc::clone(&values);
        let progress = Progress::new(Arc::new(AtomicBool::new(false)), move |value, _| {
            seen.lock().unwrap().push(value);
        });
        progress.part(0.8, 0.1).report(0.5, "step").unwrap();
        progress
            .part(0.8, 0.1)
            .part(0.5, 0.5)
            .report(1.0, "nested")
            .unwrap();
        progress.report(7.0, "clamped").unwrap();

        let values = values.lock().unwrap();
        let expected = [0.85, 0.9, 1.0];
        assert_eq!(values.len(), expected.len());
        for (value, expected) in values.iter().zip(expected) {
            assert!((value - expected).abs() < 1e-12, "{value} != {expected}");
        }
    }

    #[test]
    fn cancellation() {
        let flag = Arc::new(AtomicBool::new(false));
        let progress = Progress::new(Arc::clone(&flag), |_, _| {});
        assert!(progress.check().is_ok());
        flag.store(true, Ordering::Relaxed);
        assert!(matches!(progress.check(), Err(Error::Cancelled)));
        assert!(matches!(
            progress.part(0.0, 0.5).report(0.1, "step"),
            Err(Error::Cancelled)
        ));
        assert!(Progress::default().check().is_ok());
    }
}
