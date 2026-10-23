//! Validated unit progress, independent of provider token/time estimates.
use serde::{Deserialize, Serialize};
use std::path::Path;

#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Phase {
    #[default]
    Preparing,
    Generating,
    Reviewing,
    Rendering,
    Complete,
    Failed,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Meter {
    pub schema: u32,
    pub completed: u32,
    pub total: u32,
    pub current: u32,
    pub stage: Phase,
}

impl Default for Meter {
    fn default() -> Self {
        Self {
            schema: 1,
            completed: 0,
            total: 0,
            current: 0,
            stage: Phase::Preparing,
        }
    }
}

impl Meter {
    fn valid(&self) -> bool {
        if self.schema != 1
            || self.total > 100_000
            || self.completed > self.total
            || self.current > self.total
        {
            return false;
        }
        match self.stage {
            Phase::Preparing => self.completed == 0 && self.current == 0,
            Phase::Complete => {
                self.total > 0 && self.completed == self.total && self.current == self.total
            }
            Phase::Failed => {
                (self.total == 0 && self.current == 0)
                    || (self.current > 0 && self.completed < self.current)
            }
            _ => self.total > 0 && self.current == self.completed + 1,
        }
    }
    pub fn accepts(&self, next: &Self) -> bool {
        next.valid()
            && (self.total == 0 || self.total == next.total)
            && self.completed <= next.completed
            && self.current <= next.current
            && (!matches!(self.stage, Phase::Complete | Phase::Failed) || self == next)
            && (self.current != next.current || rank(self.stage) <= rank(next.stage))
    }
    pub fn fraction(&self) -> Option<f32> {
        (self.total > 0).then(|| self.completed as f32 / self.total as f32)
    }
}

fn rank(phase: Phase) -> u8 {
    match phase {
        Phase::Preparing => 0,
        Phase::Generating => 1,
        Phase::Reviewing => 2,
        Phase::Rendering => 3,
        Phase::Complete | Phase::Failed => 4,
    }
}

pub fn read(path: &Path) -> Option<Meter> {
    use std::io::Read;
    let mut bytes = Vec::new();
    std::fs::File::open(path)
        .ok()?
        .take(4097)
        .read_to_end(&mut bytes)
        .ok()?;
    if bytes.len() > 4096 {
        return None;
    }
    let meter: Meter = serde_json::from_slice(&bytes).ok()?;
    meter.valid().then_some(meter)
}

#[derive(gpui::IntoElement)]
pub struct View {
    pub id: gpui::SharedString,
    pub meter: Option<Meter>,
    pub active: bool,
}

impl gpui::RenderOnce for View {
    fn render(self, window: &mut gpui::Window, cx: &mut gpui::App) -> impl gpui::IntoElement {
        use crate::theme::*;
        use gpui::prelude::*;
        use gpui_component::v_flex;
        let meter = self.meter.unwrap_or_default();
        let mut view = v_flex().w_full().min_w_0().gap_2();
        if meter.total > 0 {
            view = view.child(
                accessible_text(
                    (self.id.clone(), 0usize),
                    course2md::tr!(
                        "已完成 {}/{} 个单元 · {}%",
                        meter.completed,
                        meter.total,
                        meter.completed * 100 / meter.total
                    ),
                )
                .text_size(TEXT_BODY)
                .whitespace_normal(),
            );
        }
        if self.active {
            let label = match meter.stage {
                Phase::Preparing => course2md::tr!("正在准备学习讲义…").to_owned(),
                Phase::Generating => course2md::tr!("正在生成第 {} 个单元", meter.current),
                Phase::Reviewing => course2md::tr!("正在校核第 {} 个单元", meter.current),
                Phase::Rendering => course2md::tr!("正在排版第 {} 个单元", meter.current),
                Phase::Complete => course2md::tr!("正在保存讲义结果…").to_owned(),
                Phase::Failed => course2md::tr!("本次讲义生成已停止").to_owned(),
            };
            view = view.child(
                accessible_text((self.id.clone(), 1usize), label)
                    .text_size(TEXT_BODY)
                    .text_color(color(GRAY))
                    .whitespace_normal(),
            );
        }
        if let Some(fraction) = meter.fraction() {
            view = view.child(crate::motion::progress(self.id, fraction, window, cx));
        } else if self.active {
            view = view.child(crate::motion::indeterminate_progress(self.id, cx));
        }
        view
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn unit(stage: Phase, completed: u32, current: u32) -> Meter {
        Meter {
            total: 3,
            completed,
            current,
            stage,
            ..Default::default()
        }
    }
    #[test]
    fn completion_counts_only_finished_units_and_rejects_regressions() {
        let preparing = Meter::default();
        let draft = unit(Phase::Generating, 0, 1);
        let review = unit(Phase::Reviewing, 0, 1);
        assert!(preparing.accepts(&draft));
        assert!(draft.accepts(&review));
        assert_eq!(review.fraction(), Some(0.));
        assert!(!review.accepts(&draft));
        let next = unit(Phase::Generating, 1, 2);
        assert!(review.accepts(&next));
        assert!(!next.accepts(&review));
        assert!((next.fraction().unwrap() - 1. / 3.).abs() < 0.001);
        let complete = unit(Phase::Complete, 3, 3);
        assert!(next.accepts(&complete));
        assert_eq!(complete.fraction(), Some(1.));
        assert!(!complete.accepts(&next));
        assert!(!next.accepts(&Meter {
            total: 9,
            ..next.clone()
        }));
    }
    #[test]
    fn corrupt_oversized_and_impossible_snapshots_cannot_publish_progress() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("progress.json");
        for value in [
            "{",
            &" ".repeat(5000),
            r#"{"schema":1,"completed":4,"total":3,"current":3,"stage":"complete"}"#,
        ] {
            std::fs::write(&path, value).unwrap();
            assert!(read(&path).is_none());
        }
        let review = unit(Phase::Reviewing, 1, 2);
        std::fs::write(&path, serde_json::to_vec(&review).unwrap()).unwrap();
        assert_eq!(read(&path), Some(review));
    }
}
