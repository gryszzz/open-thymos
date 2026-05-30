//! Deterministic mock cognition for tests and offline demos.
//!
//! `MockCognition` replays a scripted sequence of Intent batches. Each call to
//! `step` returns the next batch. When the script is exhausted the gateway
//! returns an empty step with an optional `final_answer`.

use thymos_core::{error::Result, intent::Intent};

use crate::{Cognition, CognitionContext, CognitionStep, CognitionUsage};

pub struct MockCognition {
    script: std::vec::IntoIter<Vec<Intent>>,
    final_answer: Option<String>,
    /// Usage reported on every non-terminal step. Defaults to zero; set via
    /// [`MockCognition::with_usage_per_step`] to exercise budget enforcement.
    usage_per_step: CognitionUsage,
}

impl MockCognition {
    pub fn new(script: Vec<Vec<Intent>>, final_answer: Option<String>) -> Self {
        MockCognition {
            script: script.into_iter(),
            final_answer,
            usage_per_step: CognitionUsage::default(),
        }
    }

    /// Report `usage` on each scripted (non-terminal) step.
    pub fn with_usage_per_step(mut self, usage: CognitionUsage) -> Self {
        self.usage_per_step = usage;
        self
    }
}

impl Cognition for MockCognition {
    fn step(&mut self, _ctx: &CognitionContext<'_>) -> Result<CognitionStep> {
        match self.script.next() {
            Some(intents) => Ok(CognitionStep {
                intents,
                final_answer: None,
                usage: self.usage_per_step,
            }),
            None => Ok(CognitionStep {
                intents: vec![],
                final_answer: self.final_answer.take(),
                usage: CognitionUsage::default(),
            }),
        }
    }
}
