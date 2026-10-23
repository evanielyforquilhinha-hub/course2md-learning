//! One start intent continues source preparation through to a readable note.
use super::*;

/// Spacing between workbench sections and the inset of its panels.
const SECTION_GAP: Rems = rems(24. / 14.);
/// Square icon buttons inside the selected video row.
const ROW_CONTROL: Rems = rems(32. / 14.);
/// Option labels and the selected video title.
const TEXT_LEAD: Rems = rems(16. / 14.);

/// A bounded pane owns its wheel offset; sibling inputs and headings stay put.
fn workbench_scroll(
    id: &'static str,
    content: impl IntoElement,
    scroll: &ScrollHandle,
) -> Stateful<Div> {
    div()
        .id(id)
        .relative()
        .w_full()
        .flex_1()
        .min_w_0()
        .min_h_0()
        .overflow_hidden()
        .child(
            v_flex()
                .id((id, 0usize))
                .w_full()
                .h_full()
                .min_w_0()
                .min_h_0()
                .overflow_y_scroll()
                .track_scroll(scroll)
                .child(div().w_full().min_w_0().flex_shrink_0().child(content)),
        )
        .child(crate::backend::vertical_scrollbar(scroll))
}
use crate::{motion, preferences::ServicePurpose, theme::*};
use anyhow::Context as _;
use course2md::config::TranscriptSource;
use course2md::subtitle::{SubtitleEvidence, SubtitleReadError, SubtitleTrack};
use gpui_component::{
    checkbox::Checkbox,
    input::{Textarea, TextareaState},
    menu::{DropdownMenu, PopupMenuItem},
    switch::Switch,
};
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};

pub(crate) struct StudyImportState {
    pub(crate) draft_id: Option<String>,
    pub(crate) prompt: Entity<TextareaState>,
    pub(crate) model: Entity<InputState>,
    pub(crate) loading: bool,
    pub(crate) error: Option<String>,
    models: crate::model_discovery::State,
    catalog_revision: u64,
    show_prompt: bool,
}

pub(crate) struct ProofreadImportState {
    pub(crate) model: Entity<InputState>,
    pub(crate) binding: Option<(String, String)>,
    pub(crate) loading: bool,
    active: bool,
    models: crate::model_discovery::State,
    catalog_revision: u64,
}

impl ProofreadImportState {
    pub(crate) fn new(model: Entity<InputState>) -> Self {
        Self {
            model,
            binding: None,
            loading: false,
            active: false,
            models: Default::default(),
            catalog_revision: 0,
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum ImportAiStage {
    Proofread,
    Study,
}

impl StudyImportState {
    pub(crate) fn new(prompt: Entity<TextareaState>, model: Entity<InputState>) -> Self {
        Self {
            draft_id: None,
            prompt,
            model,
            loading: false,
            error: None,
            models: Default::default(),
            catalog_revision: 0,
            show_prompt: false,
        }
    }
}

fn import_study_catalog_matches(
    draft: Option<&workspace::Draft>,
    draft_id: &str,
    service_id: &str,
) -> bool {
    draft.is_some_and(|draft| {
        draft.id == draft_id
            && draft.study.enabled
            && draft.study.service.as_deref() == Some(service_id)
    })
}

fn enable_study_plan(
    plan: &mut workspace::StudyPlan,
    default_prompt: &str,
    service: Option<(&str, &str)>,
) {
    plan.enabled = true;
    if plan.prompt.is_empty() {
        plan.prompt = default_prompt.to_owned();
    }
    if plan.service.is_none() {
        plan.service = service.map(|(id, _)| id.to_owned());
    }
    if plan.model.as_deref().is_none_or(str::is_empty) {
        plan.model = service.map(|(_, model)| model.to_owned());
    }
}

/// A group heading in the processing panel's single field column.
fn import_option_section(
    id: impl Into<ElementId>,
    label: &'static str,
    content: impl IntoElement,
) -> Div {
    v_flex()
        .w_full()
        .min_w_0()
        .gap_2()
        .child(theme::section_title(id, label))
        .child(content)
}

/// A plain field label above its control.
fn import_option_label(
    id: impl Into<ElementId>,
    label: &'static str,
    content: impl IntoElement,
) -> Div {
    v_flex()
        .w_full()
        .min_w_0()
        .gap_2()
        .child(
            accessible_text(id, label)
                .text_size(TEXT_AUX)
                .text_color(color(GRAY)),
        )
        .child(content)
}

/// A 16:9 cover slot; the width scales with the text size.
pub(crate) fn cover_slot(width: Rems, cover: Option<PathBuf>) -> Div {
    div()
        .w(width)
        .aspect_ratio(16. / 9.)
        .flex_shrink_0()
        .rounded(RADIUS_SMALL)
        .overflow_hidden()
        .bg(color(SECONDARY_CONTAINER))
        .flex()
        .items_center()
        .justify_center()
        .when_some(cover, |slot, cover| {
            slot.child(
                img(cover)
                    .size_full()
                    .rounded(RADIUS_SMALL)
                    .object_fit(ObjectFit::Cover),
            )
        })
}

/// Secondary text below a control.
pub(crate) fn aux_text(content: impl IntoElement) -> Div {
    div()
        .text_size(TEXT_AUX)
        .text_color(color(GRAY))
        .child(content)
}

fn import_option_switch_row(
    label: &'static str,
    hint: &'static str,
    switch: impl IntoElement,
) -> Div {
    h_flex()
        .w_full()
        .min_w_0()
        .min_h(rems(44. / 14.))
        .items_center()
        .gap_4()
        .child(
            v_flex()
                .flex_1()
                .min_w_0()
                .gap_1()
                .child(div().text_size(TEXT_LEAD).child(label))
                .when(!hint.is_empty(), |view| view.child(aux_text(hint))),
        )
        .child(div().flex_shrink_0().child(switch))
}

/// The first line of a message when it is already readable. A raw download
/// tool line (`WARNING:` / `ERROR:`) is not, and the caller supplies a summary.
fn readable_first_line(message: &str) -> Option<&str> {
    let first = message.lines().next().unwrap_or(message);
    (!first.contains("WARNING:") && !first.contains("ERROR:")).then_some(first)
}

fn help(text: impl Into<SharedString>) -> Div {
    let text = text.into();
    crate::theme::supporting_info(text_id("help", &text), text).text_sm()
}
fn issue(message: impl Into<SharedString>) -> Stateful<Div> {
    let message = message.into();
    error_text(text_id("issue", &message), message)
        .text_sm()
        .whitespace_normal()
}
fn text_id(kind: &str, value: &str) -> SharedString {
    use std::hash::{Hash, Hasher};
    let mut hash = std::collections::hash_map::DefaultHasher::new();
    value.hash(&mut hash);
    format!("import-{kind}-{:x}", hash.finish()).into()
}

/// One AI processing switch in the processing options.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ConversionAiOption {
    Proofread,
    Vision,
    Summary,
}

pub(crate) fn conversion_ai_option_label(option: ConversionAiOption) -> &'static str {
    match option {
        ConversionAiOption::Proofread => course2md::tr!("校对文字和标点"),
        ConversionAiOption::Vision => course2md::tr!("校对时附带对应截图"),
        ConversionAiOption::Summary => course2md::tr!("在开头生成摘要"),
    }
}

/// Shared explanations for the workbench and generation defaults.
pub(crate) fn conversion_ai_option_hint(
    option: ConversionAiOption,
    proofreading: bool,
) -> &'static str {
    match option {
        ConversionAiOption::Proofread => {
            course2md::tr!("修正识别错误、口头填充和标点，保留老师的原意与讲解顺序。")
        }
        ConversionAiOption::Summary => {
            course2md::tr!("依据字幕提炼概述、要点和时间大纲；可单独开启，不会逐页扩写讲解。")
        }
        ConversionAiOption::Vision if proofreading => {
            course2md::tr!("把对应截图与文字一起发送，辅助校对公式和术语；模型须支持图片。")
        }
        ConversionAiOption::Vision => {
            course2md::tr!("先开启文字校对，截图辅助才会生效；摘要仅使用字幕。")
        }
    }
}

/// Vision alone is a saved preference, not an independently requested AI step.
pub(crate) fn conversion_ai_status(proofreading: bool, summary: bool) -> &'static str {
    match (proofreading, summary) {
        (false, false) => course2md::tr!("未开启 AI 处理：生成转录原稿"),
        (true, false) => course2md::tr!("已开启：文字校对"),
        (false, true) => course2md::tr!("已开启：字幕摘要"),
        (true, true) => course2md::tr!("已开启：文字校对与字幕摘要"),
    }
}

pub(crate) fn conversion_ai_option_enabled(
    options: &ConversionOptions,
    option: ConversionAiOption,
) -> bool {
    match option {
        ConversionAiOption::Proofread => options.llm,
        ConversionAiOption::Vision => options.vision,
        ConversionAiOption::Summary => options.summarize,
    }
}

pub(crate) fn apply_conversion_ai_option(
    options: &mut ConversionOptions,
    option: ConversionAiOption,
    enabled: bool,
) {
    match option {
        ConversionAiOption::Proofread => options.llm = enabled,
        ConversionAiOption::Vision => options.vision = enabled,
        ConversionAiOption::Summary => options.summarize = enabled,
    }
}

fn text_source_choices() -> [(TranscriptSource, &'static str); 3] {
    [
        (TranscriptSource::Auto, course2md::tr!("自动")),
        (TranscriptSource::Subtitle, course2md::tr!("仅字幕")),
        (TranscriptSource::Asr, course2md::tr!("语音识别")),
    ]
}

fn text_source_id(mode: TranscriptSource) -> &'static str {
    match mode {
        TranscriptSource::Auto => "auto",
        TranscriptSource::Subtitle => "subtitle",
        TranscriptSource::Asr => "asr",
    }
}

/// A shared heading for related conversion options. 图标由调用方显式给出（不做文案子串匹配）。
pub(crate) fn box_section(icon: Icon, label: &'static str) -> Div {
    v_flex().w_full().min_w_0().gap_3().child(
        h_flex()
            .gap_2()
            .items_center()
            .child(icon.size(px(20.)).text_color(color(ACCENT)))
            .child(
                accessible_text(text_id("box-section", label), label)
                    .text_size(TEXT_TITLE)
                    .font_weight(FontWeight::SEMIBOLD),
            ),
    )
}
/// A consequence or recovery fact kept beside its action.
pub(crate) fn conversion_fact(value: impl Into<SharedString>, warning: bool) -> Stateful<Div> {
    let value = value.into();
    accessible_text(text_id("conversion-fact", &value), value)
        .w_full()
        .min_w_0()
        .whitespace_normal()
        .text_size(TEXT_BODY)
        .text_color(color(if warning { WARNING } else { INK }))
        .when(warning, |row| row.font_weight(FontWeight::SEMIBOLD))
}

fn uses_speech(source: &source::Source, source_mode: TranscriptSource) -> bool {
    source_mode == TranscriptSource::Asr
        || (source_mode == TranscriptSource::Auto
            && source.selected_subtitle.is_none()
            && source.subtitle_request.is_none()
            && source.subtitle_read_error.is_none()
            && matches!(
                source.subtitles,
                SubtitleEvidence::NoneFound
                    | SubtitleEvidence::Unsupported { .. }
                    | SubtitleEvidence::Failed { .. }
            ))
}

fn automatic_subtitle_fallback(
    source: &mut source::Source,
    source_mode: TranscriptSource,
    error: Option<&str>,
) -> bool {
    if source_mode != TranscriptSource::Auto
        || source.selected_subtitle.is_some()
        || matches!(
            source.subtitle_read_error,
            Some(SubtitleReadError::Cancelled)
        )
    {
        return false;
    }
    let failure = error.map(str::to_owned).or_else(|| {
        source
            .subtitle_read_error
            .as_ref()
            .map(ToString::to_string)
            .or_else(|| match &source.subtitles {
                SubtitleEvidence::Failed { message } => Some(message.clone()),
                _ => None,
            })
    });
    let Some(message) = failure else {
        return false;
    };
    source.subtitles = SubtitleEvidence::Failed { message };
    source.subtitle_request = None;
    source.subtitle_read_error = None;
    true
}

fn subtitle_needs_confirmation(source: &source::Source, source_mode: TranscriptSource) -> bool {
    !uses_speech(source, source_mode)
        && (source.selected_subtitle.is_none()
            || source.subtitle_request.is_some()
            || source.subtitle_read_error.is_some())
}

/// Follow only this input's submitted recovery chain. A matching URL or the
/// selected queue item alone cannot establish that a task belongs to this form.
fn submitted_input_task<'a>(
    draft: &workspace::Draft,
    tasks: &'a [workspace::TaskRecord],
    input: &str,
) -> Option<&'a workspace::TaskRecord> {
    if draft.input != input || input.trim().is_empty() {
        return None;
    }
    let find = |id: &str| {
        let mut matches = tasks.iter().filter(|task| task.id == id);
        let task = matches.next()?;
        matches.next().is_none().then_some(task)
    };
    let mut task = find(draft.submitted_task.as_deref()?)?;
    let source_id = &task.plan.source_id;
    let online = task.plan.source.online;
    if source_id.is_empty()
        || task.plan.source.input != input
        || online != draft.online
        || draft
            .source
            .as_ref()
            .is_some_and(|source| source.identity != *source_id || source.online != online)
    {
        return None;
    }
    let mut visited = std::collections::BTreeSet::new();
    loop {
        if !visited.insert(task.id.as_str())
            || task.plan.source_id != *source_id
            || task.plan.source.identity != *source_id
            || task.plan.source.online != online
        {
            return None;
        }
        let Some(next) = task.handled_by.as_deref() else {
            return Some(task);
        };
        let next = find(next)?;
        if next.parent.as_deref() != Some(task.id.as_str()) {
            return None;
        }
        task = next;
    }
}

fn completed_input_task<'a>(
    draft: &workspace::Draft,
    tasks: &'a [workspace::TaskRecord],
    input: &str,
) -> Option<&'a workspace::TaskRecord> {
    submitted_input_task(draft, tasks, input).filter(|task| {
        matches!(
            task.state,
            workspace::TaskState::Complete | workspace::TaskState::Partial
        ) && task.artifact.is_some()
    })
}

/// A saved result is history when entering the workbench. A running or paused
/// recovery, an uncertain request, and any changes after submission stay in place.
fn retired_input_task<'a>(
    draft: &workspace::Draft,
    tasks: &'a [workspace::TaskRecord],
    input: &str,
) -> Option<&'a workspace::TaskRecord> {
    let task = submitted_input_task(draft, tasks, input)?;
    if !task.state.finished()
        || task.has_uncertain()
        || (task.state != workspace::TaskState::Cancelled && task.artifact.is_none())
    {
        return None;
    }
    let submitted = tasks
        .iter()
        .find(|task| Some(task.id.as_str()) == draft.submitted_task.as_deref())?;
    (draft.title == submitted.plan.title
        && draft.options == submitted.plan.options
        && draft.library_id == submitted.plan.library_id
        && draft.folder == submitted.plan.folder
        && draft.subtitle == submitted.plan.subtitle
        && draft
            .asr_service
            .as_ref()
            .is_none_or(|id| Some(id) == submitted.plan.asr_service.as_ref())
        && draft
            .ai_service
            .as_ref()
            .is_none_or(|id| Some(id) == submitted.plan.ai_service.as_ref())
        && draft
            .ai_model
            .as_ref()
            .is_none_or(|model| model == &submitted.plan.config.llm.model))
    .then_some(task)
}

/// A live navigation intent is separate from the saved task. Leaving its
/// originating input or note ends following without cancelling the work.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum ConversionFollow {
    Preparing(u64),
    Task {
        id: String,
        source_revision: u64,
    },
    ReaderTask {
        id: String,
        base_version: PathBuf,
        reader_revision: u64,
    },
}

impl ConversionFollow {
    /// Opening a viewer owns the old version until the person closes it. Do not
    /// restore this intent afterwards, even if an in-flight read is still pending.
    pub(crate) fn interrupt_for_reader_viewer(follow: &mut Option<Self>) {
        if matches!(follow, Some(Self::ReaderTask { .. })) {
            *follow = None;
        }
    }

    pub(crate) fn submitted(self, revision: u64, id: String) -> Option<Self> {
        matches!(self, Self::Preparing(current) if current == revision).then_some(Self::Task {
            id,
            source_revision: revision,
        })
    }

    pub(crate) fn follows(&self, id: &str, revision: u64) -> bool {
        matches!(self, Self::Task { id: followed, source_revision }
            if followed == id && *source_revision == revision)
    }

    pub(crate) fn reader_reprocess(
        task: &workspace::TaskRecord,
        version: &std::path::Path,
        revision: u64,
    ) -> Option<Self> {
        let course2md::execution::Operation::Reprocess {
            base_version_dir,
            components,
            ..
        } = &task.plan.operation
        else {
            return None;
        };
        (base_version_dir == version && !components.is_empty() && !task.exports_only()).then(|| {
            Self::ReaderTask {
                id: task.id.clone(),
                base_version: base_version_dir.clone(),
                reader_revision: revision,
            }
        })
    }

    /// Check both before starting an automatic read and after its asynchronous
    /// load. The latter also retains the caller's existing read-generation guard.
    pub(crate) fn context_is_current(
        &self,
        page: Page,
        source_revision: u64,
        reader_revision: u64,
        version: Option<&std::path::Path>,
    ) -> bool {
        match self {
            Self::Preparing(_) => false,
            Self::Task {
                source_revision: expected,
                ..
            } => page == Page::New && *expected == source_revision,
            Self::ReaderTask {
                base_version,
                reader_revision: expected,
                ..
            } => {
                page == Page::Result
                    && *expected == reader_revision
                    && version == Some(base_version.as_path())
            }
        }
    }

    pub(crate) fn completed_reader_task(
        &self,
        task: &workspace::TaskRecord,
        page: Page,
        reader_revision: u64,
        version: Option<&std::path::Path>,
    ) -> bool {
        let Self::ReaderTask {
            id, base_version, ..
        } = self
        else {
            return false;
        };
        task.id == *id
            && self.context_is_current(page, 0, reader_revision, version)
            && Self::reader_reprocess(task, base_version, reader_revision).as_ref() == Some(self)
            && matches!(
                task.state,
                workspace::TaskState::Complete | workspace::TaskState::Partial
            )
            && task
                .artifact
                .as_ref()
                .is_some_and(|path| path != base_version)
    }

    pub(crate) fn completed_task<'a>(
        &self,
        on_workbench: bool,
        revision: u64,
        draft: &workspace::Draft,
        tasks: &'a [workspace::TaskRecord],
        input: &str,
    ) -> Option<&'a workspace::TaskRecord> {
        let task = completed_input_task(draft, tasks, input)?;
        (on_workbench && self.follows(&task.id, revision)).then_some(task)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ConversionGate {
    Wait,
    Stop,
    Submit,
}

/// Normal asynchronous stages retain the first Start intent. Only an abandoned
/// source or an unresolved material choice ends this automatic continuation.
/// What a pending Start depends on.
#[derive(Clone, Copy)]
struct GateInputs {
    requested_revision: Option<u64>,
    current_revision: u64,
    reading: bool,
    has_source: bool,
    needs_text_choice: bool,
    environment_ready: bool,
    library_ready: bool,
    existing_note: bool,
}

fn conversion_gate(gate: GateInputs) -> ConversionGate {
    if gate.requested_revision != Some(gate.current_revision) {
        ConversionGate::Stop
    } else if gate.reading {
        ConversionGate::Wait
    } else if !gate.has_source {
        ConversionGate::Stop
    } else if gate.needs_text_choice || !gate.environment_ready || !gate.library_ready {
        ConversionGate::Wait
    } else if gate.existing_note {
        ConversionGate::Stop
    } else {
        ConversionGate::Submit
    }
}

impl Desktop {
    /// Explicit confirmation of an already visible preview owns submission.
    pub fn start_conversion(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.reading && self.page == Page::New {
            self.read_generation = self.read_generation.wrapping_add(1);
            self.reading = false;
        }
        let input = self.value(Field::Source, cx);
        if input != self.last_source_input {
            if !self.prepare_next_import(&input, window, cx) {
                return;
            }
            self.following_conversion = None;
            self.last_source_input = input.clone();
            self.invalidate_source();
        }
        if input.is_empty() {
            self.source_validation = Some(if self.online {
                course2md::tr!("先粘贴视频链接").into()
            } else {
                course2md::tr!("先选择一个视频").into()
            });
            if self.online {
                self.inputs[&Field::Source].update(cx, |input, cx| input.focus(window, cx));
            }
            cx.notify();
            return;
        }
        // Preview is a separate user step. Never queue an unseen source,
        // including callers returning from login or selecting a local file.
        if self.source_preview.is_none() {
            self.inspect_source(window, cx);
            return;
        }
        self.pending_conversion = Some(self.preview_generation);
        self.following_conversion = (self.page == Page::New)
            .then_some(ConversionFollow::Preparing(self.preview_generation));
        self.advance_conversion(window, cx);
    }

    pub fn advance_conversion_when_ready(&mut self, cx: &mut Context<Self>) {
        if self.pending_conversion.is_none() {
            return;
        }
        let Some(handle) = cx.windows().first().copied() else {
            return;
        };
        let desktop = cx.entity().downgrade();
        cx.defer(move |cx| {
            let _ = cx.update_window(handle, |_, window, cx| {
                let _ = desktop.update(cx, |desktop, cx| desktop.advance_conversion(window, cx));
            });
        });
    }

    pub fn advance_conversion(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(generation) = self.pending_conversion else {
            return;
        };
        let reading = self.preview_cancel.is_some() || self.subtitle_loading;
        if generation == self.preview_generation
            && !reading
            && let Some(source) = &mut self.source_preview
            && automatic_subtitle_fallback(
                source,
                self.task_options.source_mode,
                self.subtitle_error.as_deref(),
            )
        {
            self.subtitle_error = None;
            if !self.save_current_draft(cx) {
                self.pending_conversion = None;
                return;
            }
        }
        match conversion_gate(GateInputs {
            requested_revision: self.pending_conversion,
            current_revision: self.preview_generation,
            reading,
            has_source: self.source_preview.is_some(),
            needs_text_choice: self.subtitle_attention_required(),
            environment_ready: self.environment.is_some(),
            library_ready: !self.loading,
            existing_note: self.existing_source_note().is_some(),
        }) {
            ConversionGate::Wait => cx.notify(),
            ConversionGate::Stop => {
                self.pending_conversion = None;
                cx.notify();
            }
            ConversionGate::Submit => self.enqueue_current(window, cx),
        }
    }

    fn subtitle_issue(&self, kind: &str, message: &str, cx: &mut Context<Self>) -> Div {
        let summary = if source::legacy_login_failure_from_text(message) {
            course2md::tr!("字幕需要登录，或改用语音识别")
        } else {
            readable_first_line(message)
                .unwrap_or(course2md::tr!("无法读取字幕，可重试或改用语音识别"))
        };
        let details = if summary != message { message } else { "" };
        let id = text_id(kind, message);
        let expanded = self.expanded_subtitle_issue.as_ref() == Some(&id);
        v_flex()
            .min_w_0()
            .gap_2()
            .child(help(summary.to_owned()))
            .when(!details.trim().is_empty(), |view| {
                view.child(
                    quiet(id.clone())
                        .icon(if expanded {
                            crate::icons::chevron_up()
                        } else {
                            crate::icons::chevron_down()
                        })
                        .self_start()
                        .label(if expanded {
                            course2md::tr!("收起技术详情")
                        } else {
                            course2md::tr!("技术详情")
                        })
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.expanded_subtitle_issue =
                                if expanded { None } else { Some(id.clone()) };
                            cx.notify();
                        })),
                )
                .child(motion::disclosure(
                    text_id("subtitle-technical", message),
                    expanded,
                    v_flex().child(help(details.to_owned())),
                ))
            })
    }

    fn import_base_config(&self) -> course2md::settings::ConfigFile {
        self.workspace
            .as_ref()
            .and_then(|workspace| workspace.state.draft())
            .and_then(|draft| draft.base_config.clone())
            .unwrap_or_else(|| self.preferences.defaults_config())
    }

    pub fn confirm_subtitle(
        &mut self,
        track: SubtitleTrack,
        explicit: bool,
        cx: &mut Context<Self>,
    ) {
        let Some(source) = self.source_preview.clone() else {
            return;
        };
        let generation = self.stop_subtitle_read();
        let source_generation = self.preview_generation;
        let token = self
            .workspace
            .as_ref()
            .and_then(|workspace| workspace.state.draft())
            .map(|draft| (draft.id.clone(), draft.revision));
        let source_id = source.identity.clone();
        let cancel = Arc::new(AtomicBool::new(false));
        self.subtitle_cancel = Some(cancel.clone());
        self.subtitle_loading = true;
        self.subtitle_error = None;
        if explicit {
            // The requested text source takes effect before I/O. A failed
            // caption read must never silently keep the previous ASR choice.
            self.task_options.source_mode = TranscriptSource::Subtitle;
        }
        if let Some(source) = &mut self.source_preview {
            source.subtitle_request = Some(track.clone());
            source.subtitle_read_error = None;
        }
        if !self.save_current_draft(cx) {
            self.subtitle_loading = false;
            self.subtitle_cancel = None;
            return;
        }
        self.preview_workers += 1;
        let task = crate::spawn_blocking_io(move || source::read_subtitle(&source, &track, cancel));
        cx.spawn(async move |this, cx| {
            let result = task.recv().await.unwrap_or_else(|_| {
                Err(course2md::subtitle::SubtitleReadError::Failed {
                    message: course2md::tr!("字幕读取意外中断").into(),
                })
            });
            let _ = this.update(cx, |this, cx| {
                this.preview_workers = this.preview_workers.saturating_sub(1);
                if this.subtitle_generation != generation
                    || this.preview_generation != source_generation
                {
                    return;
                }
                if !this
                    .source_preview
                    .as_ref()
                    .is_some_and(|source| source.identity == source_id)
                {
                    return;
                }
                if let Some((id, revision)) = &token
                    && !this
                        .workspace
                        .as_ref()
                        .is_some_and(|workspace| workspace.state.matches_input(id, *revision))
                {
                    return;
                }
                this.subtitle_loading = false;
                this.subtitle_cancel = None;
                match result {
                    Ok(subtitle) => {
                        if let Some(source) = &mut this.source_preview {
                            source.selected_subtitle = Some(subtitle);
                            source.subtitle_request = None;
                            source.subtitle_read_error = None;
                        }
                        if explicit {
                            this.task_options.source_mode = TranscriptSource::Subtitle;
                        }
                        this.save_current_draft(cx);
                    }
                    Err(SubtitleReadError::Cancelled) => {
                        this.pending_conversion = None;
                    }
                    Err(error) => {
                        this.subtitle_error = Some(error.to_string());
                        if let Some(source) = &mut this.source_preview {
                            source.subtitle_read_error = Some(error);
                        }
                        this.save_current_draft(cx);
                    }
                }
                this.advance_conversion_when_ready(cx);
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    fn choose_attached_subtitle(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(source) = self.source_preview.clone() else {
            return;
        };
        let source_generation = self.preview_generation;
        let prompt = cx.prompt_for_paths(PathPromptOptions {
            files: true,
            directories: false,
            multiple: false,
            prompt: Some(course2md::tr!("选择 SRT 或 VTT 字幕").into()),
        });
        cx.spawn_in(window, async move |this, cx| {
            let result = prompt.await;
            let _ = this.update_in(cx, |this, _, cx| {
                if this.preview_generation != source_generation
                    || !this
                        .source_preview
                        .as_ref()
                        .is_some_and(|current| current.identity == source.identity)
                {
                    return;
                }
                match result {
                    Ok(Ok(Some(paths))) => {
                        if let Some(path) = paths.into_iter().next() {
                            if !course2md::subtitle::is_subtitle_file(&path) {
                                this.subtitle_error =
                                    Some(course2md::tr!("请选择 SRT 或 VTT 字幕文件").into());
                            } else {
                                let track = course2md::subtitle::file_track(path, None);
                                if let Some(source) = &mut this.source_preview {
                                    match &mut source.subtitles {
                                        SubtitleEvidence::Found { tracks, .. } => {
                                            if !tracks
                                                .iter()
                                                .any(|candidate| candidate.id == track.id)
                                            {
                                                tracks.push(track.clone());
                                            }
                                        }
                                        _ => {
                                            source.subtitles = SubtitleEvidence::Found {
                                                tracks: vec![track.clone()],
                                                warning: None,
                                            }
                                        }
                                    }
                                }
                                this.confirm_subtitle(track, true, cx);
                            }
                        }
                    }
                    Ok(Ok(None)) => {}
                    Ok(Err(error)) => {
                        this.subtitle_error = Some(course2md::tr!(
                            "无法打开字幕选择器：{}",
                            format!("{error:#}")
                        ))
                    }
                    Err(error) => this.subtitle_error = Some(error.to_string()),
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn retry_subtitles(&mut self, cx: &mut Context<Self>) {
        let Some(source) = self.source_preview.clone() else {
            return;
        };
        let generation = self.stop_subtitle_read();
        let source_generation = self.preview_generation;
        let identity = source.identity.clone();
        let cancel = Arc::new(AtomicBool::new(false));
        self.subtitle_cancel = Some(cancel.clone());
        self.subtitle_loading = true;
        self.subtitle_error = None;
        if let Some(source) = &mut self.source_preview {
            source.subtitle_read_error = Some(SubtitleReadError::Failed {
                message: course2md::tr!("字幕重新读取未完成").into(),
            });
        }
        if !self.save_current_draft(cx) {
            self.subtitle_loading = false;
            self.subtitle_cancel = None;
            return;
        }
        let token = self
            .workspace
            .as_ref()
            .and_then(|workspace| workspace.state.draft())
            .map(|draft| (draft.id.clone(), draft.revision));
        self.preview_workers += 1;
        let task = crate::spawn_blocking_io(move || source::refresh_subtitles(&source, cancel));
        cx.spawn(async move |this, cx| {
            let result = task
                .recv()
                .await
                .unwrap_or_else(|_| Err(anyhow::anyhow!(course2md::tr!("字幕读取意外中断"))));
            let _ = this.update(cx, |this, cx| {
                this.preview_workers = this.preview_workers.saturating_sub(1);
                if this.subtitle_generation != generation
                    || this.preview_generation != source_generation
                    || token.as_ref().is_some_and(|(id, revision)| {
                        !this
                            .workspace
                            .as_ref()
                            .is_some_and(|workspace| workspace.state.matches_input(id, *revision))
                    })
                    || !this
                        .source_preview
                        .as_ref()
                        .is_some_and(|source| source.identity == identity)
                {
                    return;
                }
                this.subtitle_loading = false;
                this.subtitle_cancel = None;
                match result {
                    Ok(mut evidence) => {
                        if let SubtitleEvidence::Found { tracks, .. } = &mut evidence {
                            let original = this
                                .source_preview
                                .as_ref()
                                .and_then(|source| source.original_language.as_deref());
                            course2md::subtitle::sort_tracks(
                                tracks,
                                &this.preferences.generation().preferred_subtitle_languages,
                                "zh-Hans",
                                original,
                            );
                        }
                        let old_id = this
                            .source_preview
                            .as_ref()
                            .and_then(|source| source.selected_subtitle.as_ref())
                            .map(|subtitle| subtitle.track_id.clone());
                        let track = match &old_id {
                            Some(id) => evidence.tracks().iter().find(|track| &track.id == id),
                            None => evidence.tracks().first(),
                        }
                        .cloned();
                        if old_id.is_some() && track.is_none() {
                            this.subtitle_error = Some(course2md::tr!("原字幕无法重新读取").into());
                        }
                        if let Some(source) = &mut this.source_preview {
                            source.subtitles = evidence;
                            source.subtitle_read_error = this
                                .subtitle_error
                                .clone()
                                .map(|message| SubtitleReadError::Failed { message });
                        }
                        this.save_current_draft(cx);
                        if let Some(track) = track {
                            this.confirm_subtitle(track, false, cx);
                        }
                    }
                    Err(error) => {
                        this.subtitle_error = Some(format!("{error:#}"));
                        if let Some(source) = &mut this.source_preview {
                            source.subtitle_read_error = this
                                .subtitle_error
                                .clone()
                                .map(|message| SubtitleReadError::Failed { message });
                        }
                        this.save_current_draft(cx);
                    }
                }
                this.advance_conversion_when_ready(cx);
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    /// Cancel any subtitle read in flight and ignore its late result.
    fn stop_subtitle_read(&mut self) -> u64 {
        if let Some(cancel) = self.subtitle_cancel.take() {
            cancel.store(true, Ordering::Relaxed);
        }
        self.subtitle_generation = self.subtitle_generation.wrapping_add(1);
        self.subtitle_loading = false;
        self.subtitle_generation
    }

    /// Stop reading subtitles and drop the unfinished request.
    fn abort_subtitle_read(&mut self) {
        self.stop_subtitle_read();
        if let Some(source) = &mut self.source_preview {
            source.subtitle_request = None;
            source.subtitle_read_error = None;
        }
    }

    fn use_speech(&mut self, cx: &mut Context<Self>) {
        self.abort_subtitle_read();
        self.subtitle_error = None;
        self.task_options.source_mode = TranscriptSource::Asr;
        self.save_current_draft(cx);
        self.advance_conversion_when_ready(cx);
        cx.notify();
    }

    fn use_confirmed_subtitle(&mut self, cx: &mut Context<Self>) {
        if !self
            .source_preview
            .as_ref()
            .is_some_and(|source| source.selected_subtitle.is_some())
        {
            return;
        }
        self.abort_subtitle_read();
        self.subtitle_error = None;
        self.task_options.source_mode = TranscriptSource::Subtitle;
        self.save_current_draft(cx);
        self.advance_conversion_when_ready(cx);
        cx.notify();
    }

    fn select_source_input(&mut self, input: String, window: &mut Window, cx: &mut Context<Self>) {
        self.inputs[&Field::Source].update(cx, |state, cx| state.set_value(input, window, cx));
        self.inspect_source(window, cx);
    }

    fn toggle_candidate(&mut self, input: &str, cx: &mut Context<Self>) {
        if !self.selected_candidates.remove(input) {
            self.selected_candidates.insert(input.to_owned());
        }
        cx.notify();
    }

    fn set_candidate_selected(&mut self, input: &str, selected: bool, cx: &mut Context<Self>) {
        if selected {
            self.selected_candidates.insert(input.to_owned());
        } else {
            self.selected_candidates.remove(input);
        }
        cx.notify();
    }

    fn drop_source_files(
        &mut self,
        files: &[PathBuf],
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.online {
            self.switch_source_kind(false, window, cx);
        }
        match files {
            [path] => {
                self.inputs[&Field::Source].update(cx, |state, cx| {
                    state.set_value(path.display().to_string(), window, cx)
                });
                self.inspect_source(window, cx);
            }
            [] => {}
            _ => {
                self.invalidate_source();
                self.source_collection_title = Some(course2md::tr!("选择要处理的视频").into());
                self.source_candidates = files
                    .iter()
                    .map(|path| source::SourceCandidate {
                        input: path.display().to_string(),
                        title: path
                            .file_name()
                            .unwrap_or_default()
                            .to_string_lossy()
                            .into_owned(),
                        identity: None,
                        duration: None,
                        thumbnail: None,
                    })
                    .collect();
                cx.notify();
            }
        }
    }

    /// Source navigation is separate from the editable surface.
    fn source_kind_tabs(&self, cx: &mut Context<Self>) -> Div {
        h_flex()
            .w_full()
            .min_w_0()
            .flex_wrap()
            .items_center()
            .justify_between()
            .gap_2()
            .child(
                SingleChoiceGroup::new("source-kind", course2md::tr!("视频来源"))
                    .options([
                        ("online", course2md::tr!("视频链接")),
                        ("local", course2md::tr!("本地文件")),
                    ])
                    .icon("online", icons::link())
                    .icon("local", icons::movie())
                    .selected(if self.online { "online" } else { "local" })
                    .on_change(cx.listener(|this, value: &SharedString, window, cx| {
                        this.switch_source_kind(value.as_ref() == "online", window, cx);
                    })),
            )
            .child(
                nav_action("generation-options", self.generation_options_open, cx)
                    .icon(icons::tune())
                    .label(course2md::tr!("处理选项"))
                    .accessibility_label(course2md::tr!("处理选项"))
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.generation_options_open = !this.generation_options_open;
                        cx.notify();
                    })),
            )
    }

    pub(super) fn subtitle_attention_required(&self) -> bool {
        self.subtitle_loading
            || self.subtitle_error.is_some()
            || self.source_preview.as_ref().is_some_and(|source| {
                subtitle_needs_confirmation(source, self.task_options.source_mode)
            })
    }

    /// 选项监听器共享的收尾：草稿落盘成功后推进转换就绪，随后刷新界面。
    fn persist_task_options(&mut self, cx: &mut Context<Self>) {
        if !self.task_options.uses_cloud_provider()
            && (self.import_uses_speech()
                || (self.generation_options_open
                    && self.task_options.source_mode != TranscriptSource::Subtitle))
        {
            let (provider, model, root) = self.import_model_request();
            self.ensure_model_diagnostic(provider, Some(&model), &root, cx);
        }
        if self.save_current_draft(cx) {
            self.advance_conversion_when_ready(cx);
        }
        cx.notify();
    }

    /// Visible field, local file target, and status belonging to this source.
    fn box_source_input(&self, window: &mut Window, cx: &mut Context<Self>) -> Div {
        let mut view = v_flex().w_full().min_w_0().gap_4();
        let focused = self.inputs[&Field::Source]
            .read(cx)
            .focus_handle(cx)
            .is_focused(window);
        if self.online {
            let validation = self
                .source_preview
                .is_none()
                .then(|| self.source_validation.clone())
                .flatten();
            view = view.child(
                v_flex()
                    .gap_2()
                    // Keep the source field and its explicit preview/start action
                    // together as one compact form row.
                    .child(
                        h_flex()
                            .w_full()
                            .min_w_0()
                            .min_h(rems(52. / 14.))
                            .pl_3()
                            .pr_1()
                            .py_1()
                            .gap_2()
                            .items_center()
                            .rounded(RADIUS_SMALL)
                            .bg(color(SURFACE))
                            // The enclosing form row carries the field's focus/error ring.
                            .border_2()
                            .border_color(color(
                                if validation.is_some() || self.preview_error.is_some() {
                                    DANGER
                                } else if focused {
                                    ACCENT
                                } else {
                                    CARD_LINE
                                },
                            ))
                            .child(
                                Icon::new(icons::link())
                                    .size(ICON_SLOT)
                                    .text_color(color(GRAY)),
                            )
                            .child(
                                div().flex_1().min_w_0().child(
                                    text_input(&self.inputs[&Field::Source])
                                        .appearance(false)
                                        .aria_label(course2md::tr!("视频链接"))
                                        .w_full()
                                        .min_w_0()
                                        .px_0()
                                        .text_size(TEXT_BODY),
                                ),
                            )
                            .when(
                                validation.is_some() || self.preview_error.is_some(),
                                |row| {
                                    row.child(
                                        icons::warning().size(ICON_SLOT).text_color(color(DANGER)),
                                    )
                                },
                            )
                            // The link field retains its explicit preview action;
                            // the options toggle above is shared with local files.
                            .child(self.preview_source_button(cx)),
                    )
                    .when_some(validation, |view, message| view.child(issue(message))),
            );
        } else {
            view = self.local_source_input(view, cx);
        }
        view
    }

    /// Preview cards, collections and recovery can be long; unlike the source
    /// controls they belong in the source pane's own bounded scrolling body.
    fn box_source_details(&self, window: &mut Window, cx: &mut Context<Self>) -> Div {
        let mut view = v_flex().w_full().min_w_0().gap_4();
        if self.preview_cancel.is_some() {
            view = view.child(
                v_flex()
                    .w_full()
                    .min_w_0()
                    .gap_3()
                    .child(motion::indeterminate_progress(
                        "source-reading-progress",
                        cx,
                    ))
                    .child(
                        h_flex()
                            .w_full()
                            .min_w_0()
                            .p_4()
                            .gap_4()
                            .items_center()
                            .rounded(RADIUS_CARD)
                            .bg(color(SURFACE))
                            .child(
                                cover_slot(rems(128. / 14.), None)
                                    .max_w(relative(0.28))
                                    .bg(color(INSET)),
                            )
                            .child(
                                v_flex()
                                    .flex_1()
                                    .min_w_0()
                                    .gap_3()
                                    .child(
                                        accessible_text(
                                            "import-reading-state",
                                            course2md::tr!("正在读取视频信息…"),
                                        )
                                        .text_size(TEXT_BODY)
                                        .text_color(color(GRAY)),
                                    )
                                    .child(
                                        div()
                                            .w(relative(0.72))
                                            .h(rems(12. / 14.))
                                            .rounded_full()
                                            .bg(color(INSET)),
                                    )
                                    .child(
                                        div()
                                            .w(relative(0.45))
                                            .h(rems(10. / 14.))
                                            .rounded_full()
                                            .bg(color(INSET)),
                                    ),
                            )
                            .child(
                                quiet("cancel-source-read")
                                    .icon(crate::icons::close())
                                    .accessibility_label(course2md::tr!("取消读取视频"))
                                    .tooltip(course2md::tr!("取消读取视频"))
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.invalidate_source();
                                        cx.notify();
                                    })),
                            ),
                    ),
            );
        } else if self.source_preview.is_some() && self.workbench_linked_task(cx).is_none() {
            view = view.child(self.box_selected_video(window, cx));
        }
        if let Some(title) = &self.source_collection_title {
            view = self.candidate_list_view(view, title, cx);
        }
        view = self.source_preview_error(view, cx);
        view
    }

    /// 候选合集列表：在线来源走多选批量入队；本地多文件拖入保持单选。
    /// 身份可确认的候选才可勾选；其余保持单条打开。
    fn candidate_list_view(&self, view: Div, title: &str, cx: &mut Context<Self>) -> Div {
        let mut view = view;
        // 在线列表走多选批量入队；本地多文件拖入保持单选（本地身份要做内容哈希，
        // 逐个走原读取确认流程）。身份可确认的候选才可勾选；其余保持单条打开。
        let batch = self.online;
        // Identities are resolved once when the candidates arrive.
        let identities: Vec<Option<&str>> = self
            .source_candidates
            .iter()
            .map(|candidate| candidate.identity.as_deref())
            .collect();
        let noted: Vec<bool> = identities
            .iter()
            .map(|identity| {
                identity.is_some_and(|identity| self.notes_for_source(identity).next().is_some())
            })
            .collect();
        let selectable: Vec<String> = self
            .source_candidates
            .iter()
            .zip(&identities)
            .zip(&noted)
            .filter(|((_, identity), noted)| batch && identity.is_some() && !**noted)
            .map(|((candidate, _), _)| candidate.input.clone())
            .collect();
        let selected_count = self.selected_candidates.len();
        let all_selected = !selectable.is_empty()
            && selectable
                .iter()
                .all(|input| self.selected_candidates.contains(input));
        // 手动勾选了已有笔记的候选：入队会生成新版本，后果与操作同处可见
        let noted_selected = self
            .source_candidates
            .iter()
            .zip(&noted)
            .filter(|(candidate, noted)| {
                **noted && self.selected_candidates.contains(&candidate.input)
            })
            .count();
        let any_batchable = identities.iter().any(Option::is_some);
        let mut list = v_flex()
            .id("source-candidates")
            // 视口按字号缩放（约 3 行 + 露出第 4 行一角）；内边距让
            // 行的 hover/press 涂层不贴容器发丝边
            .max_h(rems(20.))
            .overflow_y_scroll()
            .p_1()
            .rounded(RADIUS_CARD)
            .border_1()
            .border_color(color(CARD_LINE));
        for (index, candidate) in self.source_candidates.iter().enumerate() {
            let input = candidate.input.clone();
            let untitled = candidate.title.trim().is_empty()
                || candidate.title.trim() == candidate.input.trim();
            let title = if untitled {
                course2md::tr!("视频 {}", index + 1)
            } else {
                candidate.title.clone()
            };
            let (preview, text) = self.candidate_row_parts(candidate, &title, untitled);
            let batchable = batch && identities[index].is_some();
            if batch {
                // 与导出行同一多选模式：Checkbox 自身可点（键盘 Tab+空格可切换），
                // 预览/标题区作为兄弟可点区域；行不再嵌套两层 on_click。
                // 无身份候选不可勾选，同一行样式改为箭头、点击走单条读取确认流程。
                let mut content = h_flex()
                    .id(("source-candidate-text", index))
                    .flex_1()
                    .min_w_0()
                    .gap(px(12.))
                    .items_center()
                    .cursor_pointer();
                if batchable {
                    let toggle_input = candidate.input.clone();
                    content =
                        content.on_click(cx.listener(move |this, _, _, cx| {
                            this.toggle_candidate(&toggle_input, cx)
                        }));
                } else {
                    content = content.on_click(cx.listener(move |this, _, window, cx| {
                        this.select_source_input(input.clone(), window, cx)
                    }));
                }
                content = content.child(preview).child(text);
                if batchable {
                    content = content.when(noted[index], |row| {
                        row.child(badge(BadgeKind::Success).child(course2md::tr!("已有笔记")))
                    });
                } else {
                    content = content.child(icons::arrow_forward().size(px(18.)).flex_shrink_0());
                }
                let mut row = h_flex()
                    .id(("source-candidate", index))
                    .w_full()
                    .min_w_0()
                    .gap(px(12.))
                    .items_center()
                    .py_3()
                    .px_2()
                    .rounded(RADIUS_SMALL)
                    .hover(|style| style.bg(color(INSET)));
                if batchable {
                    let selected = self.selected_candidates.contains(&candidate.input);
                    let check_input = candidate.input.clone();
                    row = row.child(
                        Checkbox::new(("source-candidate-check", index))
                            .debug_selector(move || format!("source-candidate-checkbox-{index}"))
                            .accessibility_label(course2md::tr!("选择 {}", title))
                            .checked(selected)
                            .flex_shrink_0()
                            .on_click(cx.listener(move |this, value: &bool, _, cx| {
                                this.set_candidate_selected(&check_input, *value, cx)
                            })),
                    );
                }
                list = list.child(row.child(content));
            } else {
                list = list.child(
                    quiet(("source-candidate", index))
                        .w_full()
                        .h_auto()
                        .py_3()
                        .rounded(RADIUS_SMALL)
                        .justify_start()
                        .tooltip(candidate.input.clone())
                        .accessibility_label(course2md::tr!("选择 {}", title))
                        .child(preview)
                        .child(text)
                        .child(icons::arrow_forward().size(px(18.)).flex_shrink_0())
                        .on_click(cx.listener(move |this, _, window, cx| {
                            this.select_source_input(input.clone(), window, cx)
                        })),
                );
            }
        }
        let selectable_inputs = selectable.clone();
        view = view.child(motion::enter(
            text_id("source-candidates", title),
            v_flex()
                .gap_3()
                .child(
                    accessible_text("import-collection-title", title)
                        .font_weight(FontWeight::SEMIBOLD),
                )
                .child(list)
                .when(batch && any_batchable, |view| {
                    view.child(
                        h_flex()
                            .w_full()
                            .items_center()
                            .justify_between()
                            .gap_3()
                            .child(
                                h_flex()
                                    .gap_2()
                                    .items_center()
                                    .min_w_0()
                                    .child(
                                        quiet("toggle-all-candidates")
                                            .label(if all_selected {
                                                course2md::tr!("全不选")
                                            } else {
                                                course2md::tr!("全选")
                                            })
                                            .on_click(cx.listener(move |this, _, _, cx| {
                                                if all_selected {
                                                    this.selected_candidates.clear();
                                                } else {
                                                    this.selected_candidates
                                                        .extend(selectable_inputs.iter().cloned());
                                                }
                                                cx.notify();
                                            })),
                                    )
                                    .when(noted_selected > 0, |row| {
                                        row.child(
                                            div()
                                                .min_w_0()
                                                .text_size(TEXT_AUX)
                                                .text_color(color(WARNING))
                                                .text_ellipsis()
                                                .child(course2md::tr!(
                                                    "{} 个已有笔记，将生成新版",
                                                    noted_selected
                                                )),
                                        )
                                    }),
                            )
                            .child(
                                primary_pill("start-batch-conversion")
                                    .icon(icons::arrow_forward())
                                    .label(if selected_count > 0 {
                                        course2md::tr!("整理选中的 {} 个视频", selected_count)
                                    } else {
                                        course2md::tr!("整理选中的视频").to_owned()
                                    })
                                    // 空选择即不可执行：与开始转换同一纪律
                                    .disabled(selected_count == 0)
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.enqueue_selected_candidates(cx)
                                    })),
                            ),
                    )
                }),
        ));
        view
    }

    /// 候选行共享的 leading 预览槽与标题文本：批量行与单选行仅容器与点击行为不同。
    fn candidate_row_parts(
        &self,
        candidate: &source::SourceCandidate,
        title: &str,
        untitled: bool,
    ) -> (Div, Div) {
        let cover = self.source_candidate_covers.get(&candidate.input).cloned();
        // 统一的 leading 预览槽：已缓存的首帧/封面，
        // 未就绪或无预览时回退到影片图标，保证每行文字起点对齐
        let preview = div()
            .w(rems(5.6))
            .h(rems(3.15))
            .flex_shrink_0()
            .rounded(RADIUS_SMALL)
            .overflow_hidden()
            .bg(color(INSET))
            .flex()
            .items_center()
            .justify_center()
            .when_some(cover, |slot, cover| {
                slot.child(img(cover).w_full().h_full().object_fit(ObjectFit::Cover))
            })
            .when(
                !self.source_candidate_covers.contains_key(&candidate.input),
                |slot| slot.child(icons::movie().size(px(18.)).text_color(color(GRAY))),
            );
        let text = v_flex()
            .flex_1()
            .min_w_0()
            .items_start()
            .gap_1()
            .child(
                div()
                    .w_full()
                    .min_w_0()
                    .whitespace_normal()
                    .text_ellipsis()
                    .line_clamp(2)
                    .font_weight(FontWeight::MEDIUM)
                    // 标题是行的扫描锚点：主墨色
                    .text_color(color(INK))
                    .child(title.to_owned()),
            )
            .when(untitled, |view| {
                view.child(
                    div()
                        .w_full()
                        .min_w_0()
                        .text_size(TEXT_AUX)
                        .text_color(color(GRAY))
                        .text_ellipsis()
                        .child(candidate.input.clone()),
                )
            })
            .when_some(
                candidate
                    .duration
                    .filter(|value| *value > 0. && value.is_finite() && !untitled),
                |view, seconds| {
                    view.child(
                        div()
                            .w_full()
                            .min_w_0()
                            .text_size(TEXT_AUX)
                            .text_color(color(GRAY))
                            .text_ellipsis()
                            .child(course2md::render::fmt_ts(seconds)),
                    )
                },
            );
        (preview, text)
    }

    /// Keep source failures beside the field, with retry/login/details still reachable.
    fn source_preview_error(&self, view: Div, cx: &mut Context<Self>) -> Div {
        let mut view = view;
        if let Some(error) = &self.preview_error
            && !self.source_candidates.is_empty()
        {
            view = view.child(help(error.clone()));
        }
        if let Some(error) = &self.preview_error
            && self.source_candidates.is_empty()
        {
            let details = motion::disclosure(
                "source-error-disclosure",
                self.show_preview_details,
                v_flex().child(help(error.clone())),
            );
            view = view.child(motion::enter(
                text_id("source-error", error),
                v_flex()
                    .w_full()
                    .min_w_0()
                    .gap_2()
                    .child(
                        h_flex()
                            .gap_2()
                            .items_start()
                            .child(
                                crate::icons::error()
                                    .size(px(20.))
                                    .text_color(color(DANGER))
                                    .flex_shrink_0(),
                            )
                            .child(
                                issue(if source::legacy_login_failure_from_text(error) {
                                    course2md::tr!("视频需要登录后读取，请登录后重试预览")
                                        .to_owned()
                                } else {
                                    readable_first_line(error)
                                        .unwrap_or(course2md::tr!(
                                            "暂时无法读取这个视频，请检查链接或稍后重试"
                                        ))
                                        .to_owned()
                                })
                                .flex_1()
                                .min_w_0(),
                            ),
                    )
                    .child(
                        h_flex()
                            .gap_2()
                            .flex_wrap()
                            .child(
                                outline_pill("retry-source")
                                    .icon(icons::refresh())
                                    .label(course2md::tr!("重试预览"))
                                    .on_click(cx.listener(|this, _, window, cx| {
                                        this.inspect_source(window, cx)
                                    })),
                            )
                            .when(
                                self.online
                                    && source::legacy_login_failure_from_text(error)
                                    && course2md::auth::is_bilibili_url(
                                        &self.value(Field::Source, cx),
                                    ),
                                |row| {
                                    row.child(
                                        outline_pill("source-login-repair")
                                            .icon(icons::bilibili())
                                            .label(course2md::tr!("登录 Bilibili"))
                                            .on_click(cx.listener(|this, _, window, cx| {
                                                this.open_account_dialog(window, cx)
                                            })),
                                    )
                                },
                            )
                            .child(
                                quiet("source-error-details")
                                    .icon(if self.show_preview_details {
                                        crate::icons::chevron_up()
                                    } else {
                                        crate::icons::chevron_down()
                                    })
                                    .label(if self.show_preview_details {
                                        course2md::tr!("收起技术详情")
                                    } else {
                                        course2md::tr!("技术详情")
                                    })
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.show_preview_details = !this.show_preview_details;
                                        cx.notify();
                                    })),
                            ),
                    )
                    .child(details),
            ));
        }
        view
    }

    /// 本地来源输入：空输入为拖放区，已选文件为文件名卡。
    fn local_source_input(&self, view: Div, cx: &mut Context<Self>) -> Div {
        let mut view = view;
        let input = self.value(Field::Source, cx);
        if input.is_empty() {
            view = view.child(
                h_flex()
                    .id("video-drop-zone")
                    .w_full()
                    .min_w_0()
                    .gap_3()
                    .p_4()
                    .items_center()
                    .flex_wrap()
                    .rounded(RADIUS_CARD)
                    .bg(color(INSET))
                    .border_1()
                    .border_color(color(CONTROL))
                    .child(
                        icons::file_upload()
                            .size(ICON_SLOT)
                            .text_color(color(ACCENT)),
                    )
                    .child(
                        accessible_text("import-drop-instruction", course2md::tr!("拖入本地视频"))
                            .flex_1()
                            .min_w_0()
                            .whitespace_normal()
                            .text_size(TEXT_BODY)
                            .font_weight(FontWeight::MEDIUM),
                    )
                    .child(
                        primary_pill("choose-video")
                            .icon(crate::icons::folder_open())
                            .label(course2md::tr!("选择视频"))
                            .on_click(
                                cx.listener(|this, _, window, cx| this.pick_video(window, cx)),
                            ),
                    )
                    .on_drop(
                        cx.listener(|this, paths: &gpui::ExternalPaths, window, cx| {
                            this.drop_source_files(paths.paths(), window, cx)
                        }),
                    ),
            );
        } else {
            let filename = PathBuf::from(&input)
                .file_name()
                .map(|name| name.to_string_lossy().into_owned())
                .filter(|name| !name.is_empty())
                .unwrap_or_else(|| input.clone());
            view = view.child(
                h_flex()
                    .id("selected-local-video")
                    .w_full()
                    .min_w_0()
                    .items_center()
                    .flex_wrap()
                    .gap_3()
                    .p_4()
                    .rounded(RADIUS_CARD)
                    .bg(color(SURFACE))
                    .border_1()
                    .border_color(color(HAIRLINE))
                    .child(
                        semantic_label(
                            "selected-local-video-name",
                            filename,
                            icons::movie()
                                .size(rems(20. / 14.))
                                .text_color(color(ACCENT)),
                        )
                        .flex_1()
                        .min_w(rems(18.))
                        .max_w_full(),
                    )
                    .child(
                        h_flex()
                            .gap_2()
                            .flex_wrap()
                            .items_center()
                            .child(
                                outline_pill("choose-video")
                                    .icon(crate::icons::folder_open())
                                    .label(course2md::tr!("更换视频"))
                                    .on_click(cx.listener(|this, _, window, cx| {
                                        this.pick_video(window, cx)
                                    })),
                            )
                            .when(self.can_start_input(cx), |row| {
                                row.child(self.preview_source_button(cx))
                            }),
                    )
                    .on_drop(
                        cx.listener(|this, paths: &gpui::ExternalPaths, window, cx| {
                            this.drop_source_files(paths.paths(), window, cx)
                        }),
                    ),
            );
            if self.source_preview.is_none()
                && let Some(error) = &self.source_validation
            {
                view = view.child(issue(error.clone()));
            }
        }
        view
    }

    /// The video identity remains compact; paths and account tools are details.
    fn box_selected_video(&self, window: &mut Window, cx: &mut Context<Self>) -> Div {
        let Some(source) = &self.source_preview else {
            return v_flex();
        };
        let compact = is_compact(window);
        let duration = (source.duration.is_finite() && source.duration > 0.)
            .then(|| course2md::render::fmt_ts(source.duration));
        let mut cover = cover_slot(
            rems(if compact { 128. / 14. } else { 160. / 14. }),
            source.cover.clone(),
        )
        .relative()
        .max_w(relative(0.28));
        if let Some(duration) = duration {
            cover = cover.child(
                div()
                    .absolute()
                    .right(px(4.))
                    .bottom(px(4.))
                    .px_1()
                    .py(px(1.))
                    .rounded(px(3.))
                    .bg(color(INK))
                    .text_color(color(CANVAS))
                    .text_size(rems(10. / 14.))
                    .child(duration),
            );
        }
        let pending = source.subtitle_request.is_some()
            || source.subtitle_read_error.is_some()
            || self.subtitle_error.is_some();
        let confirmed_subtitle = !self.subtitle_loading
            && !pending
            && !self.import_uses_speech()
            && source.selected_subtitle.is_some();
        let subtitle_status = if self.subtitle_loading {
            course2md::tr!("正在读取字幕…")
        } else if pending {
            course2md::tr!("文字来源待确认")
        } else if self.import_uses_speech() {
            course2md::tr!("将使用语音识别")
        } else if confirmed_subtitle {
            course2md::tr!("有字幕，将直接使用")
        } else {
            course2md::tr!("文字来源待确认")
        };
        let platform = if !source.online {
            course2md::tr!("本地视频").to_owned()
        } else {
            match course2md::config::known_video_site(&source.input) {
                Some("bilibili") => "Bilibili".to_owned(),
                Some("youtube") => "YouTube".to_owned(),
                _ => url::Url::parse(&source.input)
                    .ok()
                    .and_then(|url| url.host_str().map(str::to_owned))
                    .unwrap_or_default(),
            }
        };
        let metadata = [platform, source.author.clone()]
            .into_iter()
            .filter(|part| !part.trim().is_empty())
            .collect::<Vec<_>>()
            .join(" · ");
        let details = v_flex().gap_3().child(help(source.input.clone())).when(
            source.online && course2md::auth::is_bilibili_url(&source.input),
            |view| view.child(self.source_account_row(cx)),
        );
        let selected = h_flex()
            .w_full()
            .min_w_0()
            .gap_4()
            .items_center()
            .child(cover)
            .child(
                v_flex()
                    .min_w_0()
                    .flex_1()
                    .gap(rems(6. / 14.))
                    .child(
                        accessible_text("import-selected-title", source.title.clone())
                            .text_size(TEXT_LEAD)
                            .line_height(rems(22. / 14.))
                            .font_weight(FontWeight::SEMIBOLD)
                            .whitespace_normal()
                            .text_ellipsis()
                            .line_clamp(2),
                    )
                    .when(!metadata.is_empty(), |view| {
                        view.child(
                            div()
                                .text_size(TEXT_BODY)
                                .text_color(color(GRAY))
                                .child(metadata),
                        )
                    })
                    .child(
                        h_flex().min_w_0().child(
                            badge(BadgeKind::Neutral)
                                .rounded(RADIUS_SMALL)
                                .text_size(rems(12. / 14.))
                                .when(confirmed_subtitle, |badge| {
                                    badge.child(icons::check().size_3())
                                })
                                .child(subtitle_status),
                        ),
                    ),
            )
            .child(
                v_flex()
                    .flex_shrink_0()
                    .items_end()
                    .justify_between()
                    .gap_2()
                    .child(
                        quiet("clear-source-preview")
                            .icon(icons::close())
                            .tooltip(course2md::tr!("移除预览"))
                            .accessibility_label(course2md::tr!("移除预览"))
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.invalidate_source();
                                this.save_current_draft(cx);
                                cx.notify();
                            })),
                    )
                    .child(
                        h_flex()
                            .gap_1()
                            .child(
                                quiet("source-details")
                                    .icon(if self.show_preview_details {
                                        crate::icons::chevron_up()
                                    } else {
                                        crate::icons::chevron_down()
                                    })
                                    .w(ROW_CONTROL)
                                    .h(ROW_CONTROL)
                                    .px_0()
                                    .accessibility_label(course2md::tr!("来源详情"))
                                    .tooltip(course2md::tr!("来源详情"))
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.show_preview_details = !this.show_preview_details;
                                        cx.notify();
                                    })),
                            )
                            .when(self.online, |row| {
                                row.child(
                                    quiet("reread-source")
                                        .icon(icons::refresh())
                                        .w(ROW_CONTROL)
                                        .h(ROW_CONTROL)
                                        .px_0()
                                        .accessibility_label(course2md::tr!("重新读取"))
                                        .tooltip(course2md::tr!("重新读取"))
                                        .on_click(cx.listener(|this, _, window, cx| {
                                            this.inspect_source(window, cx)
                                        })),
                                )
                            }),
                    ),
            );
        v_flex().w_full().min_w_0().child(
            v_flex()
                .id("import-selected-source")
                .w_full()
                .min_w_0()
                .p(rems(12. / 14.))
                .rounded(RADIUS_CARD)
                .bg(color(SURFACE))
                .border_1()
                .border_color(color(CARD_LINE))
                .gap_3()
                .on_drop(
                    cx.listener(|this, paths: &gpui::ExternalPaths, window, cx| {
                        this.drop_source_files(paths.paths(), window, cx)
                    }),
                )
                .child(selected)
                .when(!self.online && self.can_start_input(cx), |card| {
                    card.child(
                        h_flex()
                            .w_full()
                            .justify_end()
                            .child(self.box_bottom_row(cx)),
                    )
                })
                .child(motion::disclosure(
                    "source-details-disclosure",
                    self.show_preview_details,
                    details,
                )),
        )
    }

    fn text_source_view(&self, cx: &mut Context<Self>) -> Div {
        let Some(source) = &self.source_preview else {
            return v_flex();
        };
        let speech = self.import_uses_speech();
        let failure = self.subtitle_error.as_deref().or_else(|| {
            if subtitle_needs_confirmation(source, self.task_options.source_mode)
                && let SubtitleEvidence::Failed { message } = &source.subtitles
            {
                Some(message.as_str())
            } else {
                None
            }
        });
        let needs_text_choice =
            subtitle_needs_confirmation(source, self.task_options.source_mode) || failure.is_some();
        let description = if self.subtitle_loading {
            course2md::tr!("正在读取字幕…").to_owned()
        } else if !speech
            && (source.subtitle_request.is_some() || source.subtitle_read_error.is_some())
        {
            match &source.selected_subtitle {
                Some(_) => course2md::tr!("字幕还没读取完成").into(),
                None => course2md::tr!("字幕还没读取完成").into(),
            }
        } else if speech {
            course2md::tr!("语音识别").into()
        } else if let Some(subtitle) = &source.selected_subtitle {
            // 自动选择的解析结果就是当前值，与显式选择同样只显示字幕名
            subtitle.label.clone()
        } else {
            match &source.subtitles {
                SubtitleEvidence::Unchecked => course2md::tr!("未检查字幕").into(),
                SubtitleEvidence::Found { .. } => course2md::tr!("请选择字幕").into(),
                SubtitleEvidence::NoneFound | SubtitleEvidence::Unsupported { .. } => {
                    course2md::tr!("尚未选择文字来源").into()
                }
                SubtitleEvidence::Failed { .. } => course2md::tr!("字幕还没读取完成").into(),
            }
        };
        let mut view = v_flex().w_full().min_w_0().gap_3().child(
            h_flex()
                .gap_3()
                .items_center()
                .child(if self.subtitle_loading {
                    motion::spinner("subtitle-reading-spinner", cx)
                } else {
                    (if speech {
                        icons::mic()
                    } else {
                        icons::subtitles()
                    })
                    .size(px(20.))
                    .text_color(color(GRAY))
                    .into_any_element()
                })
                .child(
                    accessible_text("import-text-source-state", description)
                        .font_weight(FontWeight::MEDIUM)
                        // 值与其「更换」动作收成一个对象行，不用 flex_1 把两者钉到两端
                        .min_w_0()
                        .whitespace_normal(),
                )
                .when(!needs_text_choice, |row| {
                    row.child(
                        quiet("change-text-source")
                            .icon(if self.show_options {
                                crate::icons::chevron_up()
                            } else {
                                crate::icons::chevron_down()
                            })
                            .label(if self.show_options {
                                course2md::tr!("收起")
                            } else {
                                course2md::tr!("更换")
                            })
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.show_options = !this.show_options;
                                cx.notify();
                            })),
                    )
                }),
        );
        if self.subtitle_loading {
            view = view.child(
                quiet("cancel-subtitle-read")
                    .icon(crate::icons::close())
                    .self_start()
                    .label(course2md::tr!("取消读取字幕"))
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.pending_conversion = None;
                        this.abort_subtitle_read();
                        this.save_current_draft(cx);
                        cx.notify();
                    })),
            );
        }
        if let Some(error) = failure {
            let failure_details = self.subtitle_issue("subtitle-read", error, cx);
            view = view.child(motion::enter(
                text_id("subtitle-failure", error),
                v_flex()
                    .gap_3()
                    .p_4()
                    .rounded(RADIUS_CARD)
                    .bg(color(WARNING_BG))
                    .child(
                        h_flex()
                            .gap_2()
                            .items_start()
                            .child(
                                crate::icons::warning()
                                    .size(px(20.))
                                    .text_color(color(WARNING))
                                    .flex_shrink_0(),
                            )
                            .child(
                                accessible_text(
                                    "subtitle-failed",
                                    course2md::tr!("字幕未读取成功"),
                                )
                                .font_weight(FontWeight::SEMIBOLD)
                                .text_color(color(WARNING)),
                            ),
                    )
                    .child(failure_details),
            ));
            if let Some(old) = &source.selected_subtitle {
                view = view.child(
                    outline_pill("use-previous-subtitle")
                        .self_start()
                        .icon(icons::subtitles())
                        .label(course2md::tr!("继续使用之前的字幕"))
                        .tooltip(old.label.clone())
                        .on_click(cx.listener(|this, _, _, cx| this.use_confirmed_subtitle(cx))),
                );
            }
        }
        let show_text_choices = self.show_options || needs_text_choice;
        if speech && !show_text_choices {
            match &source.subtitles {
                SubtitleEvidence::NoneFound => {
                    view = view.child(help(course2md::tr!("未找到字幕")))
                }
                SubtitleEvidence::Unsupported { message } => {
                    view = view.child(help(message.clone()))
                }
                _ => {}
            }
        }
        {
            let mut tracks = source.subtitles.tracks().to_vec();
            if let Some(pending) = &source.subtitle_request
                && !tracks.iter().any(|track| track.id == pending.id)
            {
                tracks.push(pending.clone());
            }
            let cached_id = source
                .selected_subtitle
                .as_ref()
                .map(|subtitle| subtitle.track_id.clone());
            let mut choices: Vec<_> = tracks
                .iter()
                .map(|track| (format!("track:{}", track.id), track.label()))
                .collect();
            if let Some(cached) = &source.selected_subtitle
                && !tracks.iter().any(|track| track.id == cached.track_id)
            {
                choices.push((format!("track:{}", cached.track_id), cached.label.clone()));
            }
            choices.push(("speech".into(), course2md::tr!("语音识别").into()));
            let selected = if speech {
                Some("speech".to_owned())
            } else {
                source
                    .subtitle_request
                    .as_ref()
                    .map(|track| track.id.as_str())
                    .or(cached_id.as_deref())
                    .map(|id| format!("track:{id}"))
            };
            let mut options = v_flex().gap_2().child(
                div().child(
                    SingleChoiceGroup::new("import-text-source", course2md::tr!("笔记的文字来源"))
                        .map(|group| {
                            choices.iter().fold(group, |group, (value, _)| {
                                group.icon(
                                    value.clone(),
                                    if value == "speech" {
                                        icons::mic()
                                    } else {
                                        icons::subtitles()
                                    },
                                )
                            })
                        })
                        .options(choices)
                        .when_some(selected, |group, value| group.selected(value))
                        .on_change(cx.listener(move |this, value: &SharedString, _, cx| {
                            if value.as_ref() == "speech" {
                                this.use_speech(cx);
                            } else if let Some(id) = value.strip_prefix("track:") {
                                if cached_id.as_deref() == Some(id) {
                                    this.use_confirmed_subtitle(cx);
                                } else if let Some(track) =
                                    tracks.iter().find(|track| track.id == id)
                                {
                                    this.confirm_subtitle(track.clone(), true, cx);
                                }
                            }
                        })),
                ),
            );
            if self.subtitle_error.is_none()
                && let SubtitleEvidence::Found {
                    warning: Some(message),
                    ..
                } = &source.subtitles
            {
                options = options.child(self.subtitle_issue(
                    "subtitle-warning",
                    &course2md::tr!("部分字幕无法读取：{}", message),
                    cx,
                ));
            }
            if let SubtitleEvidence::Unsupported { message } = &source.subtitles {
                options = options.child(help(message.clone()));
            }
            if matches!(source.subtitles, SubtitleEvidence::NoneFound) {
                options = options.child(help(course2md::tr!("未找到字幕")));
            }
            options = options.child(
                h_flex()
                    .gap_2()
                    .flex_wrap()
                    .child(
                        outline_pill("retry-subtitles")
                            .icon(icons::refresh())
                            .label(course2md::tr!("重新读取字幕"))
                            .disabled(self.subtitle_loading)
                            .on_click(cx.listener(|this, _, _, cx| this.retry_subtitles(cx))),
                    )
                    .when(!source.online, |view| {
                        view.child(
                            outline_pill("attach-subtitles")
                                .icon(icons::file_upload())
                                .label(course2md::tr!("选择字幕文件"))
                                .on_click(cx.listener(|this, _, window, cx| {
                                    this.choose_attached_subtitle(window, cx)
                                })),
                        )
                    })
                    .when(
                        source.online
                            && course2md::auth::is_bilibili_url(&source.input)
                            && self
                                .subtitle_error
                                .as_deref()
                                .or(match &source.subtitles {
                                    SubtitleEvidence::Failed { message } => Some(message.as_str()),
                                    _ => None,
                                })
                                .is_some_and(source::legacy_login_failure_from_text),
                        |view| {
                            view.child(
                                outline_pill("login-for-subtitles")
                                    .icon(icons::bilibili())
                                    .label(course2md::tr!("登录 Bilibili"))
                                    .on_click(cx.listener(|this, _, window, cx| {
                                        this.open_account_dialog(window, cx)
                                    })),
                            )
                        },
                    ),
            );
            view = view.child(motion::disclosure(
                "text-source-choices",
                show_text_choices,
                options,
            ));
        }
        view
    }

    fn import_text_mode_options(&self, window: &mut Window, cx: &mut Context<Self>) -> Div {
        let selector =
            SingleChoiceGroup::new("conversion-text-mode", course2md::tr!("文字来源方式"))
                .segmented()
                .options(text_source_choices().map(|(mode, label)| (text_source_id(mode), label)))
                .icon(text_source_id(TranscriptSource::Auto), icons::auto_fix())
                .icon(
                    text_source_id(TranscriptSource::Subtitle),
                    icons::subtitles(),
                )
                .icon(text_source_id(TranscriptSource::Asr), icons::mic())
                .selected(text_source_id(self.task_options.source_mode))
                .on_change(cx.listener(|this, value: &SharedString, _, cx| {
                    let Some(mode) = text_source_choices()
                        .iter()
                        .map(|(mode, _)| *mode)
                        .find(|mode| text_source_id(*mode) == value.as_ref())
                    else {
                        return;
                    };
                    if mode == TranscriptSource::Asr {
                        this.use_speech(cx);
                    } else {
                        this.task_options.source_mode = mode;
                        this.save_current_draft(cx);
                        this.advance_conversion_when_ready(cx);
                        cx.notify();
                    }
                }));
        let mut view = v_flex()
            .w_full()
            .min_w_0()
            .gap(SECTION_GAP)
            .child(import_option_section(
                "conversion-text-mode-label",
                course2md::tr!("文字来源"),
                v_flex().w_full().gap_2().child(selector).when(
                    self.task_options.source_mode == TranscriptSource::Auto,
                    |view| {
                        view.child(aux_text(course2md::tr!(
                            "有字幕时直接用字幕，没有字幕时使用语音识别"
                        )))
                    },
                ),
            ));
        if self.task_options.source_mode != TranscriptSource::Subtitle {
            view = view.child(outlined_field_label(
                "conversion-speech-label",
                course2md::tr!("语音识别"),
                TEXT_AUX,
                self.import_speech_options(window, cx),
            ));
        }
        view
    }

    pub(crate) fn sync_study_editor(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.sync_proofread_editor(window, cx);
        let Some(draft) = self.workspace.as_ref().and_then(|w| w.state.draft()) else {
            return;
        };
        if self.study_import.draft_id.as_deref() == Some(&draft.id) {
            return;
        }
        let id = draft.id.clone();
        let prompt = draft.study.prompt.clone();
        let model = draft.study.model.clone().unwrap_or_default();
        self.study_import.loading = true;
        self.study_import
            .prompt
            .update(cx, |input, cx| input.set_value(prompt, window, cx));
        self.study_import
            .model
            .update(cx, |input, cx| input.set_value(model, window, cx));
        self.study_import.draft_id = Some(id);
        self.study_import.error = None;
        self.study_import.loading = false;
        self.study_import.models.invalidate();
        self.study_import.catalog_revision = self.study_import.catalog_revision.wrapping_add(1);
        self.study_import.show_prompt = false;
        self.fetch_import_study_models(cx);
    }

    pub(crate) fn persist_study_editor(&mut self, cx: &mut Context<Self>) {
        if self.study_import.loading {
            return;
        }
        let Some(draft) = self.workspace.as_ref().and_then(|w| w.state.draft()) else {
            return;
        };
        if self.study_import.draft_id.as_deref() != Some(&draft.id) {
            return;
        }
        self.draft_deadline = Some(Instant::now() + Duration::from_millis(350));
        self.schedule_wake(Duration::from_millis(350));
        cx.notify();
    }

    fn persist_study_plan(&mut self, plan: workspace::StudyPlan, cx: &mut Context<Self>) -> bool {
        let Some(workspace) = &mut self.workspace else {
            return false;
        };
        match workspace.transaction(|state| {
            state
                .draft_mut()
                .context(course2md::tr!("当前视频输入暂时不可用"))?
                .study = plan;
            Ok(())
        }) {
            Ok(()) => {
                self.study_import.error = None;
                self.advance_conversion_when_ready(cx);
                cx.notify();
                return true;
            }
            Err(error) => {
                self.study_import.error =
                    Some(course2md::tr!("当前输入尚未保存：{}", format!("{error:#}")));
            }
        }
        cx.notify();
        false
    }

    fn set_study_enabled(&mut self, enabled: bool, window: &mut Window, cx: &mut Context<Self>) {
        let Some(mut plan) = self
            .workspace
            .as_ref()
            .and_then(|w| w.state.draft())
            .map(|draft| draft.study.clone())
        else {
            return;
        };
        if self.study_import.draft_id.as_deref()
            == self
                .workspace
                .as_ref()
                .and_then(|w| w.state.draft())
                .map(|d| d.id.as_str())
        {
            plan.prompt = self.study_import.prompt.read(cx).value().to_string();
            plan.model = Some(self.study_import.model.read(cx).value().to_string());
        }
        if enabled {
            let standard = match crate::study_ui::standard_study_prompt() {
                Ok(prompt) => prompt,
                Err(error) => {
                    self.study_import.error = Some(format!("{error:#}"));
                    cx.notify();
                    return;
                }
            };
            let default = self
                .preferences
                .generation()
                .study_prompt
                .as_deref()
                .unwrap_or(&standard);
            let selected = self
                .selected_task_service(ServicePurpose::Ai)
                .filter(|version| {
                    !self
                        .preferences
                        .service_retired_in_snapshot(&version.service_id)
                });
            enable_study_plan(
                &mut plan,
                default,
                selected
                    .as_ref()
                    .map(|version| (version.id.as_str(), version.config.model.as_str())),
            );
            self.study_import.loading = true;
            self.study_import.prompt.update(cx, |input, cx| {
                input.set_value(plan.prompt.clone(), window, cx)
            });
            self.study_import.model.update(cx, |input, cx| {
                input.set_value(plan.model.clone().unwrap_or_default(), window, cx)
            });
            self.study_import.loading = false;
        } else {
            plan.enabled = false;
        }
        if self.persist_study_plan(plan, cx) {
            self.study_import.models.invalidate();
            self.study_import.catalog_revision = self.study_import.catalog_revision.wrapping_add(1);
            if enabled {
                self.fetch_import_study_models(cx);
            }
        }
    }

    pub(crate) fn sync_proofread_editor(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let draft = self.workspace.as_ref().and_then(|w| w.state.draft());
        let selected = self.import_ai_service(ImportAiStage::Proofread);
        let binding = draft
            .zip(selected.as_ref())
            .map(|(draft, service)| (draft.id.clone(), service.id.clone()));
        if self.proofread_import.binding == binding {
            self.sync_proofread_activity(cx);
            return;
        }
        let model = draft
            .and_then(|d| d.ai_model.clone())
            .or_else(|| selected.map(|v| v.config.model))
            .unwrap_or_default();
        self.proofread_import.loading = true;
        self.proofread_import
            .model
            .update(cx, |input, cx| input.set_value(model, window, cx));
        self.proofread_import.binding = binding;
        self.proofread_import.loading = false;
        self.proofread_import.active = self.task_options.llm || self.task_options.summarize;
        self.invalidate_import_catalog(ImportAiStage::Proofread);
        self.fetch_import_ai_models(ImportAiStage::Proofread, cx);
    }

    pub(crate) fn sync_proofread_activity(&mut self, cx: &mut Context<Self>) {
        let active = self.task_options.llm || self.task_options.summarize;
        if self.proofread_import.active == active {
            return;
        }
        self.proofread_import.active = active;
        self.invalidate_import_catalog(ImportAiStage::Proofread);
        if active {
            self.fetch_import_ai_models(ImportAiStage::Proofread, cx);
        }
    }

    pub(crate) fn persist_proofread_editor(&mut self, cx: &mut Context<Self>) {
        if self.proofread_import.loading {
            return;
        }
        self.draft_deadline = Some(Instant::now() + Duration::from_millis(350));
        self.schedule_wake(Duration::from_millis(350));
        cx.notify();
    }

    fn import_ai_options(&self, cx: &mut Context<Self>) -> Div {
        let mut choices = v_flex().w_full().min_w_0().gap_2();
        for (index, option) in [
            ConversionAiOption::Proofread,
            ConversionAiOption::Summary,
            ConversionAiOption::Vision,
        ]
        .into_iter()
        .enumerate()
        {
            choices = choices.child(import_option_switch_row(
                conversion_ai_option_label(option),
                conversion_ai_option_hint(option, self.task_options.llm),
                processing_switch(Switch::new(("import-ai-option", index)))
                    .checked(conversion_ai_option_enabled(&self.task_options, option))
                    .disabled(option == ConversionAiOption::Vision && !self.task_options.llm)
                    .on_click(cx.listener(move |this, enabled, window, cx| {
                        apply_conversion_ai_option(&mut this.task_options, option, *enabled);
                        this.persist_task_options(cx);
                        this.sync_proofread_editor(window, cx);
                    })),
            ));
        }
        if self.task_options.llm
            && self
                .import_base_config()
                .llm
                .prompt
                .as_ref()
                .is_some_and(|prompt| !prompt.trim().is_empty())
        {
            choices = choices.child(help(course2md::tr!("使用自定义校对规则")));
        }
        let mut view = v_flex()
            .w_full()
            .min_w_0()
            .gap(SECTION_GAP)
            .child(import_option_section(
                "import-ai-label",
                course2md::tr!("字幕校对与摘要"),
                choices,
            ))
            .child(aux_text(conversion_ai_status(
                self.task_options.llm,
                self.task_options.summarize,
            )));
        if self.task_options.llm || self.task_options.summarize {
            view = view.child(self.import_ai_selection(ImportAiStage::Proofread, cx));
        }
        view
    }

    fn import_study_service(&self) -> Option<&crate::preferences::ServiceVersion> {
        let draft = self.workspace.as_ref()?.state.draft()?;
        if !draft.study.enabled {
            return None;
        }
        self.preferences
            .version(draft.study.service.as_deref()?)
            .filter(|version| {
                version.config.protocol.purpose() == ServicePurpose::Ai
                    && !self
                        .preferences
                        .service_retired_in_snapshot(&version.service_id)
            })
    }

    fn import_study_catalog_current(&self, revision: u64, draft: &str, service: &str) -> bool {
        self.study_import.catalog_revision == revision
            && import_study_catalog_matches(
                self.workspace.as_ref().and_then(|w| w.state.draft()),
                draft,
                service,
            )
            && self.import_study_service().is_some_and(|v| v.id == service)
    }

    /// Model discovery reads the selected service's catalog; it never sends
    /// course material or changes the currently selected model.
    fn fetch_import_study_models(&mut self, cx: &mut Context<Self>) {
        self.fetch_import_ai_models(ImportAiStage::Study, cx);
    }

    fn import_ai_service(
        &self,
        stage: ImportAiStage,
    ) -> Option<crate::preferences::ServiceVersion> {
        match stage {
            ImportAiStage::Study => self.import_study_service().cloned(),
            ImportAiStage::Proofread => {
                self.selected_task_service(ServicePurpose::Ai).filter(|v| {
                    v.config.protocol.purpose() == ServicePurpose::Ai
                        && !self.preferences.service_retired_in_snapshot(&v.service_id)
                })
            }
        }
    }

    fn import_catalog(&self, stage: ImportAiStage) -> (&crate::model_discovery::State, u64) {
        match stage {
            ImportAiStage::Study => (
                &self.study_import.models,
                self.study_import.catalog_revision,
            ),
            ImportAiStage::Proofread => (
                &self.proofread_import.models,
                self.proofread_import.catalog_revision,
            ),
        }
    }

    fn import_catalog_mut(
        &mut self,
        stage: ImportAiStage,
    ) -> (&mut crate::model_discovery::State, &mut u64) {
        match stage {
            ImportAiStage::Study => (
                &mut self.study_import.models,
                &mut self.study_import.catalog_revision,
            ),
            ImportAiStage::Proofread => (
                &mut self.proofread_import.models,
                &mut self.proofread_import.catalog_revision,
            ),
        }
    }

    fn invalidate_import_catalog(&mut self, stage: ImportAiStage) {
        let (models, revision) = self.import_catalog_mut(stage);
        models.invalidate();
        *revision = revision.wrapping_add(1);
    }

    fn import_ai_catalog_current(
        &self,
        stage: ImportAiStage,
        revision: u64,
        draft_id: &str,
        service: &str,
    ) -> bool {
        if stage == ImportAiStage::Study {
            return self.import_study_catalog_current(revision, draft_id, service);
        }
        self.import_catalog(stage).1 == revision
            && (self.task_options.llm || self.task_options.summarize)
            && self
                .workspace
                .as_ref()
                .and_then(|w| w.state.draft())
                .is_some_and(|d| d.id == draft_id)
            && self
                .import_ai_service(stage)
                .is_some_and(|v| v.id == service)
    }

    fn fetch_import_ai_models(&mut self, stage: ImportAiStage, cx: &mut Context<Self>) {
        if self.import_catalog(stage).0.loading()
            || (stage == ImportAiStage::Proofread
                && !self.task_options.llm
                && !self.task_options.summarize)
        {
            return;
        }
        let Some(version) = self.import_ai_service(stage) else {
            return;
        };
        let Some(draft_id) = self
            .workspace
            .as_ref()
            .and_then(|w| w.state.draft())
            .map(|d| d.id.clone())
        else {
            return;
        };
        let (_, generation) = self.import_catalog_mut(stage);
        *generation = generation.wrapping_add(1);
        let revision = *generation;
        let selected = version.id.clone();
        if version.config.protocol == crate::preferences::ServiceProtocol::CodexResponses {
            self.import_catalog_mut(stage).0.begin_external();
            let worker =
                crate::spawn_blocking_io(course2md::login::codex::refresh_models_for_desktop);
            cx.spawn(async move |this, cx| {
                let result = worker.recv().await;
                let _ = this.update(cx, |this, cx| {
                    if !this.import_ai_catalog_current(stage, revision, &draft_id, &selected) {
                        return;
                    }
                    match result {
                        Ok(Ok(models)) => this
                            .import_catalog_mut(stage)
                            .0
                            .prime(models.into_iter().map(|(id, _)| id).collect()),
                        _ => this.import_catalog_mut(stage).0.reject(
                            crate::model_discovery::Error::Custom(
                                course2md::tr!(
                                    "无法获取订阅模型列表，请检查账号；当前模型保持不变。"
                                )
                                .into(),
                            ),
                        ),
                    }
                    cx.notify();
                });
            })
            .detach();
        } else {
            let request = crate::model_discovery::Request::from_draft(
                &crate::preferences::ServiceDraft::from_version(&version),
                crate::credentials::Secret::new(""),
            );
            let request = match request {
                Ok(request) => request,
                Err(error) => {
                    self.import_catalog_mut(stage).0.reject(error);
                    cx.notify();
                    return;
                }
            };
            let ticket = self.import_catalog_mut(stage).0.begin(&request);
            let vault = self.preferences.vault();
            cx.spawn(async move |this, cx| {
                let result = crate::model_discovery::discover(request, vault).await;
                let _ = this.update(cx, |this, cx| {
                    if !this.import_ai_catalog_current(stage, revision, &draft_id, &selected) {
                        return;
                    }
                    let current = this.import_ai_service(stage).and_then(|version| {
                        crate::model_discovery::RequestKey::from_draft(
                            &crate::preferences::ServiceDraft::from_version(&version),
                            "",
                        )
                        .ok()
                    });
                    this.import_catalog_mut(stage)
                        .0
                        .complete(ticket, current.as_ref(), result);
                    cx.notify();
                });
            })
            .detach();
        }
        cx.notify();
    }

    fn select_study_service(&mut self, id: &str, window: &mut Window, cx: &mut Context<Self>) {
        let Some(version) = self
            .preferences
            .version(id)
            .filter(|version| {
                version.config.protocol.purpose() == ServicePurpose::Ai
                    && !self
                        .preferences
                        .service_retired_in_snapshot(&version.service_id)
            })
            .cloned()
        else {
            return;
        };
        let Some(mut plan) = self
            .workspace
            .as_ref()
            .and_then(|w| w.state.draft())
            .map(|draft| draft.study.clone())
        else {
            return;
        };
        plan.prompt = self.study_import.prompt.read(cx).value().to_string();
        plan.service = Some(version.id);
        plan.model = Some(version.config.model);
        self.study_import.loading = true;
        self.study_import.model.update(cx, |input, cx| {
            input.set_value(plan.model.clone().unwrap_or_default(), window, cx)
        });
        self.study_import.loading = false;
        if self.persist_study_plan(plan, cx) {
            self.study_import.models.invalidate();
            self.study_import.catalog_revision = self.study_import.catalog_revision.wrapping_add(1);
            self.fetch_import_study_models(cx);
        }
    }

    /// Both stages use the same field composition; service editing stays in settings.
    fn import_ai_selection(&self, stage: ImportAiStage, cx: &mut Context<Self>) -> Div {
        let index = stage as usize;
        let selected = self.import_ai_service(stage);
        let selected_id = selected.as_ref().map(|v| v.id.clone());
        let mut services = self
            .preferences
            .latest_versions()
            .into_values()
            .filter(|v| {
                v.config.protocol.purpose() == ServicePurpose::Ai
                    && !self.preferences.service_retired_in_snapshot(&v.service_id)
            })
            .collect::<Vec<_>>();
        // A pinned older configuration remains selectable without silently upgrading it.
        if let Some(version) = &selected
            && !services.iter().any(|v| v.id == version.id)
        {
            services.insert(0, version.clone());
        }
        let label = match stage {
            ImportAiStage::Proofread => course2md::tr!("校对与摘要服务"),
            ImportAiStage::Study => course2md::tr!("讲义服务"),
        };
        let model_label = match stage {
            ImportAiStage::Proofread => course2md::tr!("校对与摘要模型"),
            ImportAiStage::Study => course2md::tr!("讲义模型"),
        };
        let name = selected
            .as_ref()
            .map(|v| v.config.name.clone())
            .unwrap_or_else(|| course2md::tr!("请选择 AI 服务").into());
        let draft_id = self
            .workspace
            .as_ref()
            .and_then(|w| w.state.draft())
            .map(|d| d.id.clone());
        let weak = cx.weak_entity();
        let picker = control(("import-ai-service", index))
            .w_full()
            .min_w_0()
            .h_auto()
            .min_h(CONTROL_HEIGHT)
            .py_2()
            .accessibility_label(course2md::tr!("{}：{}", label, name))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .whitespace_normal()
                    .font_weight(FontWeight::NORMAL)
                    .child(name),
            )
            .child(icons::chevron_down().size_4().flex_shrink_0())
            .dropdown_menu(move |menu, _, _| {
                services
                    .iter()
                    .fold(menu.scrollable(true), |menu, version| {
                        let id = version.id.clone();
                        let name = version.config.name.clone();
                        let model = version.config.model.clone();
                        let current = selected_id.as_ref() == Some(&id);
                        let weak = weak.clone();
                        let draft_id = draft_id.clone();
                        menu.item(
                            PopupMenuItem::element(move |_, _| {
                                v_flex()
                                    .min_w_0()
                                    .gap_1()
                                    .child(name.clone())
                                    .child(aux_text(model.clone()))
                            })
                            .checked(current)
                            .on_click(move |_, window, cx| {
                                let _ = weak.update(cx, |this, cx| {
                                    if this
                                        .workspace
                                        .as_ref()
                                        .and_then(|w| w.state.draft())
                                        .map(|d| &d.id)
                                        != draft_id.as_ref()
                                    {
                                        return;
                                    }
                                    if stage == ImportAiStage::Study {
                                        this.select_study_service(&id, window, cx);
                                    } else if this.preferences.version(&id).is_some_and(|v| {
                                        v.config.protocol.purpose() == ServicePurpose::Ai
                                            && !this
                                                .preferences
                                                .service_retired_in_snapshot(&v.service_id)
                                    }) && this.save_current_draft(cx)
                                        && this.bind_service(
                                            ServicePurpose::Ai,
                                            Some(id.clone()),
                                            true,
                                            cx,
                                        )
                                    {
                                        this.proofread_import.binding = None;
                                        this.sync_proofread_editor(window, cx);
                                    }
                                });
                            }),
                        )
                    })
            });
        let model = match stage {
            ImportAiStage::Proofread => &self.proofread_import.model,
            ImportAiStage::Study => &self.study_import.model,
        };
        let mut fields = v_flex()
            .w_full()
            .min_w_0()
            .gap_3()
            .child(import_option_label(
                ("import-ai-service-label", index),
                label,
                picker,
            ))
            .child(import_option_label(
                ("import-ai-model-label", index),
                model_label,
                crate::model_discovery::model_field(
                    if stage == ImportAiStage::Study {
                        "import-study-model"
                    } else {
                        "import-proofread-model"
                    },
                    model,
                    self.import_catalog(stage).0,
                    selected.is_none(),
                    None,
                    cx.listener(move |this, _, _, cx| this.fetch_import_ai_models(stage, cx)),
                    cx,
                ),
            ));
        if let Some(selected) = selected {
            fields = fields.child(aux_text(
                crate::settings_ui::service_form::ai_service_billing_hint(selected.config.protocol),
            ));
            fields = fields.child(aux_text(course2md::tr!(
                "{}会发送到 {}",
                if stage == ImportAiStage::Study
                    || (self.task_options.llm && self.task_options.vision)
                {
                    course2md::tr!("文字及对应截图")
                } else {
                    course2md::tr!("文字")
                },
                selected.config.host()
            )));
        } else {
            fields = fields.child(issue(course2md::tr!(
                "请选择已保存的 AI 服务，再开始转换。"
            )));
        }
        fields.child(
            quiet(("import-ai-manage-services", index))
                .self_start()
                .icon(icons::settings())
                .label(course2md::tr!("管理服务与模型"))
                .on_click(cx.listener(|this, _, window, cx| {
                    this.open_settings(window, cx);
                    this.select_settings_tab(crate::settings_ui::SettingsTab::Services, window, cx);
                })),
        )
    }

    fn import_study_options(&self, cx: &mut Context<Self>) -> Div {
        let plan = self
            .workspace
            .as_ref()
            .and_then(|w| w.state.draft())
            .map(|draft| &draft.study);
        let enabled = plan.is_some_and(|plan| plan.enabled);
        let mut fields = v_flex()
            .w_full()
            .min_w_0()
            .gap_3()
            .child(import_option_switch_row(
                course2md::tr!("转录完成后生成学习讲义"),
                course2md::tr!("结合截图和你的提示词生成分步讲解，可与字幕校对分别开启。"),
                processing_switch(Switch::new("import-study-enabled"))
                    .checked(enabled)
                    .on_click(cx.listener(|this, enabled, window, cx| {
                        this.set_study_enabled(*enabled, window, cx)
                    })),
            ));
        if enabled {
            fields = fields
                .child(self.import_ai_selection(ImportAiStage::Study, cx))
                .child(
                    quiet("import-study-edit-prompt")
                        .self_start()
                        .icon(if self.study_import.show_prompt {
                            icons::chevron_up()
                        } else {
                            icons::edit()
                        })
                        .label(if self.study_import.show_prompt {
                            course2md::tr!("收起教学提示词")
                        } else {
                            course2md::tr!("编辑教学提示词")
                        })
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.study_import.show_prompt = !this.study_import.show_prompt;
                            cx.notify();
                        })),
                )
                .when(self.study_import.show_prompt, |fields| {
                    fields.child(import_option_label(
                        "import-study-prompt-label",
                        course2md::tr!("本次教学提示词"),
                        Textarea::new(&self.study_import.prompt)
                            .w_full()
                            .h(px(240.))
                            .aria_label(course2md::tr!("本次教学提示词")),
                    ))
                })
                .child(aux_text(course2md::tr!(
                    "本次提示词只用于这个课程，不会修改默认教学提示词。"
                )));
        }
        if let Some(error) = &self.study_import.error {
            fields = fields.child(issue(error.clone()));
        }
        import_option_section("import-study-label", course2md::tr!("AI 学习讲义"), fields)
    }

    fn reset_task_ai_overrides(&mut self, cx: &mut Context<Self>) {
        if !self.save_current_draft(cx) {
            return;
        }
        let defaults = ConversionOptions::from_config(&self.preferences.defaults_config());
        let Some(workspace) = &mut self.workspace else {
            return;
        };
        if let Err(error) = workspace.transaction(|state| {
            state
                .draft_mut()
                .context(course2md::tr!("当前视频输入暂时不可用"))?
                .reset_ai_overrides(&defaults);
            Ok(())
        }) {
            self.workspace_error = Some(course2md::tr!(
                "本次选项尚未恢复默认：{}",
                format!("{error:#}")
            ));
            cx.notify();
            return;
        }
        self.task_options.llm = defaults.llm;
        self.task_options.summarize = defaults.summarize;
        self.task_options.vision = defaults.vision;
        self.sync_proofread_activity(cx);
        self.advance_conversion_when_ready(cx);
        cx.notify();
    }

    fn import_speech_options(&self, window: &mut Window, cx: &mut Context<Self>) -> Div {
        let cloud = self.task_options.uses_cloud_provider();
        let label = if cloud {
            course2md::tr!("语音识别服务").to_owned()
        } else {
            let (_, model, _) = self.import_model_request();
            course2md::tr!("本机 · {}", course2md::config::model_label(&model))
        };
        let engine = if cloud {
            String::new()
        } else {
            self.local_engine_name().to_owned()
        };
        let current = self.task_options.provider;
        let entity = cx.entity().downgrade();
        let providers: Vec<_> = providers()
            .iter()
            .filter_map(|(id, label)| {
                use course2md::config::AsrProvider;
                let provider = crate::provider_from_id(id);
                // Automatic, CPU and the cloud service are always offered.
                (matches!(provider, None | Some(AsrProvider::Cpu | AsrProvider::Api))
                    || provider == current
                    || provider
                        .zip(self.environment.as_ref())
                        .is_some_and(|(provider, environment)| environment.supports(provider)))
                .then_some((provider, *label))
            })
            .collect();
        let selector = control("import-speech-location")
            .h(rems(56. / 14.))
            .font_weight(FontWeight::NORMAL)
            .text_size(TEXT_LEAD)
            .rounded(RADIUS_SMALL)
            .w_full()
            .min_w_0()
            .accessibility_label(course2md::tr!("语音识别方式"))
            .child(div().flex_1().min_w_0().text_left().child(label))
            .child(
                div()
                    .text_color(color(GRAY))
                    .font_weight(FontWeight::NORMAL)
                    .child(engine),
            )
            .child(crate::icons::chevron_down().size_4())
            .dropdown_menu(move |menu, _, _| {
                providers.iter().fold(menu, |menu, &(provider, label)| {
                    let entity = entity.clone();
                    menu.item(
                        PopupMenuItem::new(label)
                            .checked(provider == current)
                            .on_click(move |_, _, cx| {
                                let _ = entity.update(cx, |this, cx| {
                                    this.task_options.provider = provider;
                                    this.persist_task_options(cx);
                                });
                            }),
                    )
                })
            });
        let mut view = v_flex().w_full().min_w_0().gap_2().child(selector);
        if cloud {
            view = view
                .child(self.task_service_picker(ServicePurpose::Speech, cx))
                .child(aux_text(course2md::tr!("音频会发送到所选语音识别服务")));
        } else {
            let (provider, model, root) = self.import_model_request();
            let snapshot = self.setup_model_snapshot(provider, Some(&model), &root);
            let ready = snapshot.device_issue.is_none()
                && !snapshot.preparing
                && snapshot.result.as_ref().is_some_and(|result| {
                    result.as_ref().is_ok_and(|status| {
                        matches!(
                            status.state,
                            course2md::models::status::CacheState::Cached
                                | course2md::models::status::CacheState::Loaded
                        )
                    })
                });
            if ready {
                view = view.child(aux_text(course2md::tr!(
                    "模型已下载 · {}",
                    course2md::config::model_label(&model)
                )));
            } else if snapshot.device_issue.is_none()
                && !snapshot.checking
                && !snapshot.preparing
                && snapshot.notice.is_none()
                && !snapshot.cancelled
                && snapshot.result.as_ref().is_some_and(|result| {
                    result.as_ref().is_ok_and(|status| {
                        status.state == course2md::models::status::CacheState::Missing
                            && status.can_prepare
                            && status.last_error.is_none()
                    })
                })
            {
                // A routine first download needs one status line and one action.
                // Checks, failures and active transfers retain the full recovery panel below.
                let prepare_model = model.clone();
                let prepare_root = root.clone();
                let details = cx.weak_entity();
                view = view
                    .child(
                        h_flex()
                            .w_full()
                            .min_w_0()
                            .gap_2()
                            .flex_wrap()
                            .items_center()
                            .child(
                                div()
                                    .flex_1()
                                    .min_w_0()
                                    .text_size(TEXT_AUX)
                                    .text_color(color(GRAY))
                                    .child(course2md::tr!(
                                        "{} · 待下载",
                                        course2md::config::model_label(&model)
                                    )),
                            )
                            .child(
                                outline_pill("import-prepare-model")
                                    .icon(icons::download())
                                    .label(course2md::tr!("下载模型"))
                                    .disabled(self.job.is_some())
                                    .on_click(cx.listener(move |this, _, _, cx| {
                                        this.prepare_setup_model(
                                            provider,
                                            Some(&prepare_model),
                                            &prepare_root,
                                            cx,
                                        );
                                    })),
                            )
                            .child(
                                quiet("import-model-details")
                                    .icon(icons::ellipsis())
                                    .w(CONTROL_HEIGHT)
                                    .px_0()
                                    .accessibility_label(course2md::tr!("模型检查与设置"))
                                    .tooltip(course2md::tr!("模型检查与设置"))
                                    .dropdown_menu(move |menu, _, _| {
                                        let recheck = details.clone();
                                        let settings = details.clone();
                                        menu.item(
                                            PopupMenuItem::new(course2md::tr!("重新检查模型"))
                                                .icon(icons::refresh())
                                                .on_click(move |_, _, cx| {
                                                    let _ = recheck.update(cx, |this, cx| {
                                                        this.refresh_model_diagnostics(cx)
                                                    });
                                                }),
                                        )
                                        .item(
                                            PopupMenuItem::new(course2md::tr!("模型设置"))
                                                .icon(icons::settings())
                                                .on_click(move |_, window, cx| {
                                                    let _ = settings.update(cx, |this, cx| {
                                                        this.open_settings(window, cx)
                                                    });
                                                }),
                                        )
                                    }),
                            ),
                    )
                    .when(self.job.is_some(), |view| {
                        view.child(aux_text(course2md::tr!("当前任务完成后才能下载模型")))
                    });
            } else {
                view = view.child(self.model_readiness_panel_with(
                    provider,
                    Some(&model),
                    &root,
                    false,
                    window,
                    cx,
                ));
            }
        }
        view
    }

    fn import_uses_speech(&self) -> bool {
        self.source_preview
            .as_ref()
            .is_some_and(|source| uses_speech(source, self.task_options.source_mode))
    }

    fn missing_import_service(&self) -> Option<ServicePurpose> {
        let separate_subtitle = self.task_options.source_mode != TranscriptSource::Asr
            && self
                .workspace
                .as_ref()
                .and_then(|workspace| workspace.state.draft())
                .is_some_and(|draft| draft.subtitle.is_some());
        [
            (
                ServicePurpose::Speech,
                self.task_options.uses_cloud_provider()
                    && self.import_uses_speech()
                    && !separate_subtitle,
            ),
            (
                ServicePurpose::Ai,
                self.task_options.llm || self.task_options.summarize,
            ),
        ]
        .into_iter()
        .find_map(|(purpose, required)| {
            (required
                && self.selected_task_service(purpose).is_none_or(|version| {
                    self.preferences
                        .service_retired_in_snapshot(&version.service_id)
                }))
            .then_some(purpose)
        })
    }

    fn import_model_request(&self) -> (course2md::config::AsrProvider, String, PathBuf) {
        let provider = self.actual_local_provider();
        let mut config = self.import_base_config();
        // 与计划构造同一回填规则：空白时按识别方式补默认模型（NPU 专属解析）。
        crate::task_ui::fill_default_asr_model(&mut config, provider);
        let model = config.defaults.asr_model.unwrap_or_default();
        let root = course2md::config::model_dir_from(config.defaults.model_dir.as_deref());
        (provider, model, root)
    }

    fn actual_local_provider(&self) -> course2md::config::AsrProvider {
        use course2md::config::AsrProvider;
        self.task_options
            .provider
            .filter(|provider| *provider != AsrProvider::Api)
            .unwrap_or_else(|| self.recommended_local_provider())
    }
    fn local_engine_name(&self) -> &'static str {
        crate::provider_label(Some(self.actual_local_provider()))
    }

    fn import_destination(&self, cx: &mut Context<Self>) -> Div {
        let current = self
            .workspace
            .as_ref()
            .and_then(|workspace| workspace.state.draft())
            .map(|draft| draft.library_id.clone());
        let libraries = self
            .workspace
            .as_ref()
            .map(|workspace| workspace.state.libraries.clone())
            .unwrap_or_default();
        let selected = libraries
            .iter()
            .find(|library| Some(&library.id) == current.as_ref())
            .cloned();
        let mut view = v_flex().w_full().min_w_0().gap_2();
        if libraries.len() > 1 {
            let entity = cx.entity().downgrade();
            let label = selected
                .as_ref()
                .map(|library| library.name.clone())
                .unwrap_or_else(|| course2md::tr!("选择保存位置").into());
            view = view.child(
                control("import-library")
                    .icon(crate::icons::folder_open())
                    .w_full()
                    .min_w_0()
                    .accessibility_label(course2md::tr!("保存到：{}", label))
                    .tooltip(label.clone())
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .whitespace_nowrap()
                            .text_ellipsis()
                            .child(label),
                    )
                    .child(crate::icons::chevron_down().size_4().flex_shrink_0())
                    .dropdown_menu(move |menu, _, _| {
                        libraries.iter().fold(menu, |menu, library| {
                            let id = library.id.clone();
                            let entity = entity.clone();
                            menu.item(
                                PopupMenuItem::new(format!(
                                    "{} · {}",
                                    library.name,
                                    library.root.display()
                                ))
                                .checked(current.as_ref() == Some(&id))
                                .on_click(move |_, _, cx| {
                                    let _ = entity.update(cx, |this, cx| {
                                        if !this.save_current_draft(cx) {
                                            return;
                                        }
                                        if let Some(workspace) = &mut this.workspace {
                                            match workspace.transaction(|state| {
                                                if let Some(draft) = state.draft_mut() {
                                                    draft.library_id = id.clone();
                                                    draft.folder = None;
                                                }
                                                Ok(())
                                            }) {
                                                Ok(()) => {
                                                    this.target_folder = None;
                                                    this.advance_conversion_when_ready(cx);
                                                }
                                                Err(error) => {
                                                    this.workspace_error = Some(course2md::tr!(
                                                        "保存位置尚未更新：{}",
                                                        format!("{error:#}")
                                                    ))
                                                }
                                            }
                                        }
                                        cx.notify();
                                    });
                                }),
                            )
                        })
                    }),
            );
        }
        view = view.child(
            h_flex()
                .gap_2()
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .child(self.folder_picker(None, 0, cx)),
                )
                .child(
                    outline_pill("import-new-folder")
                        .icon(icons::create_new_folder())
                        .w(CONTROL_HEIGHT)
                        .h(CONTROL_HEIGHT)
                        .px_0()
                        .accessibility_label(course2md::tr!("新建文件夹"))
                        .tooltip(course2md::tr!("新建文件夹"))
                        .on_click(
                            cx.listener(|this, _, window, cx| this.begin_folder(None, window, cx)),
                        ),
                ),
        );
        v_flex()
            .w_full()
            .min_w_0()
            .when(self.source_preview.is_some(), |view| {
                view.child(import_option_label(
                    "import-title-label",
                    course2md::tr!("笔记名称"),
                    crate::focus_scroll::RevealFocus::new(
                        ("import-title-focus", self.validation_attempt),
                        text_input(&self.inputs[&Field::Title]).w_full(),
                        self.scrolls[Page::New as usize].clone(),
                    ),
                ))
            })
            .child(outlined_field_label(
                "import-destination-label",
                course2md::tr!("保存到"),
                TEXT_AUX,
                view,
            ))
    }

    fn import_exports(&self, cx: &mut Context<Self>) -> Div {
        let mut choices = h_flex().w_full().min_w_0().gap_2().flex_wrap();
        for (index, label) in ["Markdown", course2md::tr!("网页"), "JSON"]
            .into_iter()
            .enumerate()
        {
            choices = choices.child(
                filter_chip(
                    ("import-export", index),
                    self.task_options.formats[index],
                    cx,
                )
                .when(self.task_options.formats[index], |button| {
                    button.icon(icons::check())
                })
                .label(label)
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.task_options.formats[index] = !this.task_options.formats[index];
                    this.save_current_draft(cx);
                    cx.notify();
                })),
            );
        }
        v_flex()
            .w_full()
            .min_w_0()
            .child(import_option_section(
                "import-exports-label",
                course2md::tr!("同时导出"),
                choices,
            ))
            .when(self.online, |view| {
                view.child(
                    div()
                        .w_full()
                        .min_w_0()
                        .pt(SECTION_GAP)
                        .child(import_option_switch_row(
                            course2md::tr!("保留原视频"),
                            course2md::tr!("供离线播放，会占用更多磁盘空间"),
                            processing_switch(Switch::new("import-keep-video"))
                                .checked(self.task_options.keep_video)
                                .on_click(cx.listener(|this, value, _, cx| {
                                    this.task_options.keep_video = *value;
                                    this.save_current_draft(cx);
                                    cx.notify();
                                })),
                        )),
                )
            })
    }

    fn existing_source_note(&self) -> Option<&Course> {
        let source = self.source_preview.as_ref()?;
        let preferred = self
            .workspace
            .as_ref()
            .and_then(|workspace| {
                workspace
                    .state
                    .draft()
                    .and_then(|draft| workspace.state.library(&draft.library_id))
            })
            .map(|library| &library.root);
        self.notes_for_source(&source.identity)
            .min_by_key(|course| !preferred.is_some_and(|root| course.dir.starts_with(root)))
    }

    fn can_start_input(&self, cx: &App) -> bool {
        // 候选列表展示时，批量/单选操作在列表自己的行动区；输入行的「开始转换」
        // 对合集链接必然失败，不再同屏出现两个 primary
        self.source_collection_title.is_none()
            && (self.online || !self.value(Field::Source, cx).is_empty())
            && self.preview_error.is_none()
            && self.existing_source_note().is_none()
            && self.current_input_task(cx).is_none_or(|task| {
                matches!(
                    task.state,
                    workspace::TaskState::Complete | workspace::TaskState::Cancelled
                ) && !(self.reading
                    && self
                        .following_conversion
                        .as_ref()
                        .is_some_and(|follow| follow.follows(&task.id, self.preview_generation)))
            })
    }

    fn preview_source_button(&self, cx: &mut Context<Self>) -> Div {
        // Enter remains bound to inspect_source. Only this explicitly relabeled
        // button changes to conversion once an eligible preview is on screen.
        let start = self.online
            && self.source_preview.is_some()
            && self.workbench_linked_task(cx).is_none()
            && self.can_start_input(cx);
        let disabled = self.preview_cancel.is_some()
            || self.value(Field::Source, cx).is_empty()
            || (start && self.pending_conversion.is_some());
        let label = if start {
            course2md::tr!("开始转换")
        } else {
            course2md::tr!("预览视频")
        };
        h_flex().items_center().child(
            theme::disabled_when(primary_pill("preview-source"), disabled, cx)
                .h(CONTROL_HEIGHT)
                .icon(icons::arrow_forward())
                .label(label)
                .accessibility_label(label)
                .tooltip(label)
                .track_focus(&self.import_submit_focus)
                .disabled(disabled)
                .on_click(cx.listener(move |this, _, window, cx| {
                    if start {
                        this.start_conversion(window, cx);
                    } else {
                        this.inspect_source(window, cx);
                    }
                })),
        )
    }

    /// Conversion starts only from the explicit action beside the preview.
    fn box_bottom_row(&self, cx: &mut Context<Self>) -> Div {
        let disabled = self.pending_conversion.is_some()
            || self.source_preview.is_none()
            || self.preview_cancel.is_some();
        h_flex().items_center().child(
            theme::disabled_when(primary_pill("start-conversion"), disabled, cx)
                .icon(icons::arrow_forward())
                .label(course2md::tr!("开始转换"))
                .disabled(disabled)
                .on_click(cx.listener(|this, _, window, cx| this.start_conversion(window, cx))),
        )
    }

    pub(super) fn current_input_task(&self, cx: &App) -> Option<&workspace::TaskRecord> {
        let state = &self.workspace.as_ref()?.state;
        submitted_input_task(state.draft()?, &state.tasks, &self.value(Field::Source, cx))
    }

    /// The source form and preview share one definition of the submitted task.
    fn workbench_linked_task(&self, cx: &App) -> Option<&workspace::TaskRecord> {
        let opening_result = self.reading && self.following_conversion.is_some();
        self.current_input_task(cx).filter(|task| {
            (opening_result
                && matches!(
                    task.state,
                    workspace::TaskState::Complete | workspace::TaskState::Partial
                ))
                || !task.state.finished()
                || (task.state == workspace::TaskState::Partial
                    && task.artifact.is_some()
                    && !crate::task_ui::task_component_failures(task).is_empty())
        })
    }

    /// Called by explicit workbench entry and initial restoration, never render.
    /// Terminal tasks remain in the task list and course library after their input
    /// has become a fresh form; an in-flight or edited input is never replaced.
    pub(super) fn prepare_workbench_input(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        self.sync_proofread_editor(window, cx);
        if self.pending_conversion.is_some() || self.following_conversion.is_some() {
            return true;
        }
        let input = self.value(Field::Source, cx);
        let has_old_result = self.workspace.as_ref().is_some_and(|workspace| {
            workspace.state.draft().is_some_and(|draft| {
                retired_input_task(draft, &workspace.state.tasks, &input).is_some()
            })
        });
        if !has_old_result {
            return true;
        }
        if !self.save_current_draft(cx) {
            return false;
        }
        let defaults = ConversionOptions::from_config(&self.preferences.defaults_config());
        let Some(workspace) = &mut self.workspace else {
            return true;
        };
        let Some(draft) = workspace.state.draft() else {
            return true;
        };
        if retired_input_task(draft, &workspace.state.tasks, &input).is_none() {
            return true;
        }
        let destination = (draft.library_id == workspace.state.default_library)
            .then(|| {
                draft
                    .folder
                    .map(|folder| (draft.library_id.clone(), folder))
            })
            .flatten();
        match workspace.transaction(|state| {
            state.reset_input(self.online, defaults, destination);
            Ok(())
        }) {
            Ok(()) => {
                self.completed_source = None;
                self.invalidate_source();
                self.restore_draft(window, cx);
                self.source_editor_open = true;
                self.generation_options_open = true;
                true
            }
            Err(error) => {
                self.workspace_error = Some(course2md::tr!(
                    "新的视频输入尚未建立：{}",
                    format!("{error:#}")
                ));
                cx.notify();
                false
            }
        }
    }

    pub fn new_page(&mut self, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let opening_result = self.reading && self.following_conversion.is_some();
        let linked_state = self.workbench_linked_task(cx).map(|task| task.state);
        let cancelled_notice = self
            .current_input_task(cx)
            .filter(|task| task.state == workspace::TaskState::Cancelled)
            .map(|task| (task.id.clone(), course2md::tr!("任务已取消").to_owned()));
        let input = v_flex()
            .w_full()
            .min_w_0()
            .gap_4()
            .child(self.source_kind_tabs(cx))
            .child(self.box_source_input(window, cx));
        let mut view = v_flex()
            .gap(SECTION_GAP)
            .w_full()
            .min_w_0()
            .child(self.box_source_details(window, cx));
        if let Some((id, message)) = cancelled_notice {
            view = view.child(info_callout(
                SharedString::from(format!("cancelled-input-task-{id}")),
                message,
            ));
        }
        let has_linked_task = linked_state.is_some();
        // Queued and running tasks are shown by the active task list below.
        let settled_task = linked_state
            .filter(|state| {
                !matches!(
                    state,
                    workspace::TaskState::Queued
                        | workspace::TaskState::Running
                        | workspace::TaskState::Pausing
                )
            })
            .and_then(|_| self.workbench_linked_task(cx).cloned());
        if let Some(task) = settled_task {
            let task_view = match task.state {
                workspace::TaskState::Complete | workspace::TaskState::Partial
                    if opening_result =>
                {
                    self.box_task_opening_result(&task, cx)
                }
                _ => self.box_task_attention(&task, cx),
            };
            view = view.child(
                // Recovery stays beside the source that created this task.
                task_view
                    .p(rems(20. / 14.))
                    .rounded(RADIUS_CARD)
                    .bg(color(SURFACE))
                    .border_1()
                    .border_color(color(CARD_LINE))
                    .shadow(shadow_card()),
            );
        } else if !has_linked_task {
            let text_required = self.subtitle_attention_required();
            if self.source_preview.is_some() {
                let mut source = v_flex().w_full().min_w_0().gap_4();
                if text_required {
                    source = source.child(
                        box_section(icons::subtitles(), course2md::tr!("文字来源"))
                            .child(self.text_source_view(cx)),
                    );
                }
                source = source.child(self.conversion_recovery(cx));
                view = view.child(source);
            } else if text_required {
                view = view.child(
                    box_section(icons::subtitles(), course2md::tr!("文字来源"))
                        .child(self.text_source_view(cx)),
                );
            }
        }
        let scale = f32::from(window.rem_size()) / 14.;
        let column = crate::views::shell_content_width(Page::New, window);
        // Use the remaining content pane after the global sidebar, not the
        // outer window. Large type switches to a readable stacked form.
        let wide = column >= 820. * scale;
        let source_scroll = window
            .use_keyed_state("workbench-source-scroll", cx, |_, _| ScrollHandle::new())
            .read(cx)
            .clone();
        let options_scroll = self.scrolls[Page::New as usize].clone();
        let show_options = self.generation_options_open
            && self.preview_cancel.is_none()
            && self.source_candidates.is_empty()
            && self
                .current_input_task(cx)
                .is_none_or(|task| task.state.finished());
        let active = self.workbench_active_tasks_section(window, cx);
        let options = if show_options {
            Some(
                v_flex()
                    .w_full()
                    .min_w_0()
                    .p(rems(20. / 14.))
                    .gap(SECTION_GAP)
                    .rounded(RADIUS_CARD)
                    .bg(color(SURFACE))
                    .border_1()
                    .border_color(color(CARD_LINE))
                    .child(
                        v_flex()
                            .w_full()
                            .gap(rems(4. / 14.))
                            .child(
                                h_flex()
                                    .w_full()
                                    .items_center()
                                    .child(
                                        div()
                                            .flex_1()
                                            .text_size(TEXT_TITLE)
                                            .line_height(rems(26. / 14.))
                                            .font_weight(FontWeight::SEMIBOLD)
                                            .child(course2md::tr!("处理选项")),
                                    )
                                    .child(
                                        quiet("close-generation-options")
                                            .icon(crate::icons::close())
                                            .w(CONTROL_HEIGHT)
                                            .px_0()
                                            .accessibility_label(course2md::tr!("收起处理选项"))
                                            .on_click(cx.listener(|this, _, _, cx| {
                                                this.generation_options_open = false;
                                                cx.notify();
                                            })),
                                    ),
                            )
                            .child(aux_text(course2md::tr!("仅对本次生效"))),
                    )
                    .child(self.import_text_mode_options(window, cx))
                    .child(div().h(px(1.)).bg(color(HAIRLINE)))
                    .child(self.import_ai_options(cx))
                    .child(div().h(px(1.)).bg(color(HAIRLINE)))
                    .child(self.import_study_options(cx))
                    .child(div().h(px(1.)).bg(color(HAIRLINE)))
                    .child(self.import_destination(cx))
                    .child(self.import_exports(cx))
                    .child(
                        h_flex().w_full().justify_end().pt(rems(8. / 14.)).child(
                            quiet("reset-task-overrides")
                                .icon(icons::refresh())
                                .label(course2md::tr!("恢复 AI 默认"))
                                .on_click(
                                    cx.listener(|this, _, _, cx| this.reset_task_ai_overrides(cx)),
                                ),
                        ),
                    ),
            )
        } else {
            None
        };
        let view = view.when_some(active, |view, active| view.child(active));
        let mut form = div()
            .flex()
            .w_full()
            .flex_1()
            .min_h_0()
            .min_w_0()
            .gap(SECTION_GAP)
            .items_stretch();
        if wide {
            form = form.child(
                v_flex()
                    .flex_1()
                    .min_w_0()
                    .min_h_0()
                    .gap(SECTION_GAP)
                    .child(input.flex_shrink_0())
                    .child(workbench_scroll("workbench-source", view, &source_scroll)),
            );
            if let Some(options) = options {
                form = form.child(
                    div()
                        .flex()
                        .w(rems(360. / 14.))
                        .flex_shrink_0()
                        .min_h_0()
                        .child(workbench_scroll(
                            "workbench-options",
                            options,
                            &options_scroll,
                        )),
                );
            }
        } else {
            // Stacked windows keep the source input above the bounded lower
            // pane, so every option remains reachable without losing context.
            form = form
                .flex_col()
                .child(input.flex_shrink_0())
                .child(workbench_scroll(
                    "workbench-stacked",
                    view.children(options),
                    &options_scroll,
                ));
        }
        let view = v_flex()
            .w_full()
            .h_full()
            .min_w_0()
            .min_h_0()
            .pt(px(24.))
            .pb_4()
            .gap(SECTION_GAP)
            .child(
                theme::page_heading("workbench-title", course2md::tr!("添加课程")).flex_shrink_0(),
            )
            .child(form);
        // The whole workbench accepts a dropped video, not just the local tab.
        view.on_drop(
            cx.listener(|this, paths: &gpui::ExternalPaths, window, cx| {
                this.drop_source_files(paths.paths(), window, cx)
            }),
        )
        .into_any_element()
    }

    /// Attach last to the full-window `workbench-drop` group, outside scrolling content.
    pub(super) fn workbench_drop_overlay(&self, cx: &mut Context<Self>) -> Stateful<Div> {
        div()
            .id("workbench-drop-overlay")
            .absolute()
            .inset_0()
            .invisible()
            .group_drag_over::<gpui::ExternalPaths>("workbench-drop", |style| style.visible())
            .bg(color(CANVAS).opacity(0.88))
            .p(SECTION_GAP)
            .child(
                v_flex()
                    .size_full()
                    .items_center()
                    .justify_center()
                    .gap_4()
                    .rounded(RADIUS_HERO)
                    .border_2()
                    .border_dashed()
                    .border_color(color(ACCENT))
                    .child(
                        h_flex()
                            .size(rems(88. / 14.))
                            .rounded_full()
                            .items_center()
                            .justify_center()
                            .bg(color(SECONDARY_CONTAINER))
                            .child(
                                icons::file_upload()
                                    .size(rems(40. / 14.))
                                    .text_color(color(ACCENT)),
                            ),
                    )
                    .child(
                        div()
                            .text_size(rems(28. / 14.))
                            .font_weight(FontWeight::SEMIBOLD)
                            .child(course2md::tr!("松开以添加视频")),
                    ),
            )
            .on_drop(
                cx.listener(|this, paths: &gpui::ExternalPaths, window, cx| {
                    this.drop_source_files(paths.paths(), window, cx);
                }),
            )
    }

    /// Only unresolved choices interrupt a submitted conversion.
    fn conversion_recovery(&mut self, cx: &mut Context<Self>) -> Div {
        let busy = self.job.is_some()
            || self.workspace.as_ref().is_some_and(|workspace| {
                workspace.state.tasks.iter().any(|task| {
                    matches!(
                        task.state,
                        crate::workspace::TaskState::Queued | crate::workspace::TaskState::Running
                    )
                })
            });
        let reading = self.preview_cancel.is_some() || self.subtitle_loading;
        // Derived once per draft/preference/source revision, not on every render.
        let cache_key = (
            self.workspace
                .as_ref()
                .map_or(0, |workspace| workspace.state.revision),
            self.preferences
                .revision(crate::preferences::PreferenceGroup::Generation),
            self.preview_generation,
        );
        let preview_plan = match &self.preview_plan_cache {
            Some((workspace, preferences, source, plan))
                if (*workspace, *preferences, *source) == cache_key =>
            {
                plan.clone()
            }
            _ => {
                let plan = self.preview_plan().map_err(|error| format!("{error:#}"));
                self.preview_plan_cache =
                    Some((cache_key.0, cache_key.1, cache_key.2, plan.clone()));
                plan
            }
        }
        .map_err(anyhow::Error::msg);
        let matching_task = preview_plan
            .as_ref()
            .ok()
            .and_then(|plan| self.matching_task(plan))
            .map(|task| {
                let location = self.task_library(task).map(|library| library.name.clone());
                (task.id.clone(), task.plan.title.clone(), location)
            });
        let preferences_issue = self.ordinary_preferences_submit_issue();
        let preferences_blocked = preferences_issue.is_some();
        let submission_error = preferences_issue
            .as_ref()
            .map(|issue| issue.message.clone())
            .or_else(|| {
                if self.source_preview.is_some() && !reading && !self.subtitle_attention_required()
                {
                    preview_plan
                        .as_ref()
                        .err()
                        .map(|error| format!("{error:#}"))
                } else {
                    None
                }
            });
        let checking_components =
            !preferences_blocked && self.environment.is_none() && submission_error.is_some();
        let mut inset = v_flex().w_full().min_w_0().gap_3();
        let mut actions = h_flex().gap_2().flex_wrap();
        if self.environment.as_ref().is_some_and(|environment| {
            !environment.engine
                || !environment.ffmpeg
                || !environment.ffprobe
                || (self.online && !environment.ytdlp)
        }) {
            actions = actions.child(
                outline_pill("repair-conversion-components")
                    .icon(icons::settings())
                    .label(course2md::tr!("检查所需组件"))
                    .on_click(cx.listener(|this, _, window, cx| {
                        this.open_settings(window, cx);
                        this.select_settings_tab(
                            crate::settings_ui::SettingsTab::Application,
                            window,
                            cx,
                        );
                    })),
            );
        }
        if let Some(issue) = preferences_issue {
            actions = actions.child(
                outline_pill("repair-generation-preferences")
                    .icon(icons::refresh())
                    .label(if issue.can_retry {
                        course2md::tr!("重试保存处理选项")
                    } else {
                        course2md::tr!("重置处理选项")
                    })
                    .on_click(cx.listener(move |this, _, window, cx| {
                        if !this.save_current_draft(cx) {
                            return;
                        }
                        let saved = if issue.can_retry {
                            this.retry_ordinary_preferences(issue.group, cx)
                        } else {
                            this.restore_ordinary_preferences(issue.group, window, cx)
                        };
                        if saved {
                            let focus = this.import_submit_focus.clone();
                            window.defer(cx, move |window, cx| focus.focus(window, cx));
                        }
                    })),
            );
        }
        let draft_library = self.workspace.as_ref().and_then(|workspace| {
            workspace
                .state
                .draft()
                .and_then(|draft| workspace.state.library(&draft.library_id))
        });
        actions = actions.children(self.reassociate_action(
            draft_library,
            outline_pill("reassociate-import-location"),
            cx,
        ));
        if let Some((id, title, location)) = matching_task.clone() {
            let location = location.unwrap_or_else(|| course2md::tr!("原保存位置").into());
            inset = inset.child(conversion_fact(
                course2md::tr!("已有相同处理任务：《{}》（{}）", title, location),
                true,
            ));
            actions = actions.child(
                primary_pill("show-matching-task")
                    .track_focus(&self.import_submit_focus)
                    .self_start()
                    .icon(icons::task())
                    .label(course2md::tr!("查看任务"))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.save_current_draft(cx);
                        this.select_task(&id, cx);
                        this.navigate(Page::Task, cx);
                        cx.notify();
                    })),
            );
        } else if let Some(course) = self.existing_source_note().cloned() {
            let location = self
                .course_location(&course)
                .map(|library| library.name.clone())
                .unwrap_or_else(|| course2md::tr!("笔记库").into());
            inset = inset.child(info_callout(
                "existing-note-notice",
                course2md::tr!("已有笔记，保存在「{}」", location),
            ));
            actions = actions.child(
                h_flex()
                    .gap_2()
                    .flex_wrap()
                    .child(
                        primary_pill("open-existing-note")
                            .icon(crate::icons::book_open())
                            .label(course2md::tr!("打开已有笔记"))
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.save_current_draft(cx);
                                this.open_course(course.clone(), cx);
                            })),
                    )
                    .child(
                        outline_pill("generate-new-version")
                            .icon(icons::refresh())
                            .track_focus(&self.import_submit_focus)
                            .label(course2md::tr!("生成新版笔记"))
                            .disabled(reading || self.workspace.is_none() || preferences_blocked)
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.following_conversion =
                                    Some(ConversionFollow::Preparing(this.preview_generation));
                                this.enqueue_current(window, cx);
                            })),
                    ),
            );
            if busy {
                inset = inset.child(conversion_fact(course2md::tr!("将排在当前任务之后"), false));
            }
        } else if let Some(purpose) = self.missing_import_service() {
            actions = actions.child(
                primary_pill("configure-conversion-service")
                    .icon(icons::settings())
                    .label(match purpose {
                        ServicePurpose::Speech => course2md::tr!("设置语音识别服务"),
                        ServicePurpose::Ai => course2md::tr!("设置 AI 服务"),
                    })
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.open_task_service_editor(purpose, window, cx);
                    })),
            );
        }
        if self.pending_conversion.is_some()
            && self.source_validation.is_some()
            && !reading
            && self.missing_import_service().is_none()
            && self.existing_source_note().is_none()
            && matching_task.is_none()
        {
            actions = actions.child(
                outline_pill("retry-conversion-validation")
                    .icon(icons::refresh())
                    .label(course2md::tr!("重试转换"))
                    .on_click(cx.listener(|this, _, window, cx| {
                        this.start_conversion(window, cx);
                    })),
            );
        }
        if self.pending_conversion.is_some() && !reading {
            actions = actions.child(
                quiet("cancel-pending-conversion")
                    .icon(icons::close())
                    .label(course2md::tr!("取消转换"))
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.pending_conversion = None;
                        this.following_conversion = None;
                        cx.notify();
                    })),
            );
        }
        inset
            .when_some(submission_error, |view, message| {
                if checking_components {
                    view.child(
                        h_flex()
                            .min_w_0()
                            .gap_2()
                            .items_center()
                            .child(motion::spinner("import-component-check-spinner", cx))
                            .child(
                                accessible_text("import-component-check", message)
                                    .role(Role::Status)
                                    .text_sm()
                                    .font_weight(FontWeight::NORMAL)
                                    .whitespace_normal()
                                    .text_color(color(GRAY)),
                            ),
                    )
                } else {
                    view.child(
                        accessible_text("import-submit-error", message)
                            .role(Role::Alert)
                            .text_sm()
                            .whitespace_normal()
                            .text_color(color(DANGER)),
                    )
                }
            })
            .child(actions)
    }
}

#[cfg(test)]
mod tests {
    use super::{
        ConversionFollow, ConversionGate, GateInputs, automatic_subtitle_fallback,
        completed_input_task, conversion_gate, enable_study_plan, import_study_catalog_matches,
        submitted_input_task, subtitle_needs_confirmation, uses_speech,
    };
    use crate::{source, workspace};
    use course2md::config::TranscriptSource;
    use course2md::subtitle::{CachedSubtitle, SubtitleEvidence, SubtitleReadError};
    use gpui::{
        Context, InteractiveElement as _, IntoElement, ParentElement as _, Render, ScrollHandle,
        Styled as _, TestAppContext, Window, div, point, px,
    };

    #[test]
    fn enabling_study_fills_missing_choices_without_replacing_course_edits() {
        let mut plan = workspace::StudyPlan::default();
        plan.model = Some(String::new());
        enable_study_plan(
            &mut plan,
            "saved teaching default",
            Some(("version-1", "model-1")),
        );
        assert!(plan.enabled);
        assert_eq!(plan.prompt, "saved teaching default");
        assert_eq!(plan.service.as_deref(), Some("version-1"));
        assert_eq!(plan.model.as_deref(), Some("model-1"));

        plan.enabled = false;
        plan.prompt = "course-specific teaching".into();
        plan.model = Some("course-model".into());
        enable_study_plan(
            &mut plan,
            "changed future default",
            Some(("version-2", "model-2")),
        );
        assert_eq!(plan.prompt, "course-specific teaching");
        assert_eq!(plan.service.as_deref(), Some("version-1"));
        assert_eq!(plan.model.as_deref(), Some("course-model"));
    }

    #[test]
    fn study_catalog_response_belongs_to_enabled_draft_and_selected_service() {
        let mut draft = workspace::Draft::new(true, "library".into(), Default::default());
        draft.study.enabled = true;
        draft.study.service = Some("codex-version".into());
        let id = draft.id.clone();
        assert!(import_study_catalog_matches(
            Some(&draft),
            &id,
            "codex-version"
        ));
        assert!(!import_study_catalog_matches(
            Some(&draft),
            "previous-course",
            "codex-version"
        ));
        assert!(!import_study_catalog_matches(
            Some(&draft),
            &id,
            "previous-service"
        ));
        draft.study.enabled = false;
        assert!(!import_study_catalog_matches(
            Some(&draft),
            &id,
            "codex-version"
        ));
        assert!(!import_study_catalog_matches(None, &id, "codex-version"));
    }

    struct PaneHarness {
        scroll: ScrollHandle,
    }

    impl Render for PaneHarness {
        fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
            div()
                .flex()
                .flex_col()
                .w(px(900.))
                .h(px(500.))
                .child(
                    div()
                        .h(px(56.))
                        .flex_shrink_0()
                        .debug_selector(|| "fixed-heading".into()),
                )
                .child(
                    div()
                        .flex()
                        .flex_1()
                        .min_h_0()
                        .items_stretch()
                        .child(
                            div()
                                .w(px(500.))
                                .flex_shrink_0()
                                .debug_selector(|| "fixed-source".into()),
                        )
                        .child(
                            super::workbench_scroll(
                                "options-fixture",
                                div().h(px(1400.)).debug_selector(|| "long-options".into()),
                                &self.scroll,
                            )
                            .debug_selector(|| "options-viewport".into()),
                        ),
                )
        }
    }

    #[gpui::test]
    fn processing_options_scroll_without_moving_source_or_heading(cx: &mut TestAppContext) {
        let scroll = ScrollHandle::new();
        let (_view, cx) = cx.add_window_view(|_, _| PaneHarness {
            scroll: scroll.clone(),
        });
        cx.update(|window, cx| {
            window.draw(cx).clear(cx);
        });
        let heading = cx.debug_bounds("fixed-heading").unwrap();
        let source = cx.debug_bounds("fixed-source").unwrap();
        let viewport = cx.debug_bounds("options-viewport").unwrap();
        let content = cx.debug_bounds("long-options").unwrap();
        assert!(viewport.size.height <= px(444.));
        assert!(
            scroll.max_offset().y >= px(900.),
            "long options must have their own scroll extent"
        );
        scroll.set_offset(point(px(0.), px(-500.)));
        cx.update(|window, cx| {
            window.draw(cx).clear(cx);
        });
        assert_eq!(cx.debug_bounds("fixed-heading").unwrap(), heading);
        assert_eq!(cx.debug_bounds("fixed-source").unwrap(), source);
        assert_eq!(cx.debug_bounds("options-viewport").unwrap(), viewport);
        assert!(cx.debug_bounds("long-options").unwrap().top() < content.top() - px(400.));
    }

    /// A pending Start for revision 7 with every prerequisite met.
    fn ready() -> GateInputs {
        GateInputs {
            requested_revision: Some(7),
            current_revision: 7,
            reading: false,
            has_source: true,
            needs_text_choice: false,
            environment_ready: true,
            library_ready: true,
            existing_note: false,
        }
    }

    #[test]
    fn preview_completion_without_confirmation_never_submits() {
        for reading in [true, false] {
            for environment_ready in [true, false] {
                for library_ready in [true, false] {
                    let gate = GateInputs {
                        requested_revision: None,
                        reading,
                        environment_ready,
                        library_ready,
                        ..ready()
                    };
                    assert_eq!(conversion_gate(gate), ConversionGate::Stop);
                }
            }
        }
    }

    #[test]
    fn confirmed_start_waits_for_remaining_prerequisites() {
        // The same Start revision survives each independently completing prerequisite.
        for (reading, has_source, needs_text_choice, environment_ready) in [
            (true, false, false, false),
            (true, true, true, true),
            (false, true, false, false),
            (false, true, true, true),
        ] {
            let gate = GateInputs {
                reading,
                has_source,
                needs_text_choice,
                environment_ready,
                ..ready()
            };
            assert_eq!(conversion_gate(gate), ConversionGate::Wait);
        }
        assert_eq!(conversion_gate(ready()), ConversionGate::Submit);
        // Source replacement and duplicate content require a fresh intent/choice.
        let replaced = GateInputs {
            current_revision: 8,
            reading: true,
            has_source: false,
            ..ready()
        };
        assert_eq!(conversion_gate(replaced), ConversionGate::Stop);
        assert_eq!(
            conversion_gate(GateInputs {
                existing_note: true,
                ..ready()
            }),
            ConversionGate::Stop
        );
        assert_eq!(
            conversion_gate(GateInputs {
                has_source: false,
                ..ready()
            }),
            ConversionGate::Stop
        );
    }

    #[test]
    fn first_start_waits_for_library_scan_before_existing_note_decision() {
        // A restored source and the environment can be ready before the async
        // library scan replaces its initially empty (or stale) note list.
        for existing_note in [false, true] {
            let gate = GateInputs {
                library_ready: false,
                existing_note,
                ..ready()
            };
            assert_eq!(conversion_gate(gate), ConversionGate::Wait);
        }
        // The original Start is still pending when the scan reports a result.
        assert_eq!(
            conversion_gate(GateInputs {
                existing_note: true,
                ..ready()
            }),
            ConversionGate::Stop
        );
        assert_eq!(conversion_gate(ready()), ConversionGate::Submit);
        let replaced = GateInputs {
            current_revision: 8,
            library_ready: false,
            ..ready()
        };
        assert_eq!(conversion_gate(replaced), ConversionGate::Stop);
    }

    #[test]
    fn explicit_new_version_follows_new_task_instead_of_previous_result() {
        let (mut draft, mut tasks) = submitted_recovery_chain();
        tasks[0].state = workspace::TaskState::Complete;
        tasks[0].artifact = Some("versions/previous".into());
        tasks[0].handled_by = None;
        tasks[0].error = None;
        tasks[1].id = "new-version".into();
        tasks[1].parent = None;
        tasks[1].plan.operation = Default::default();
        draft.submitted_task = Some(tasks[1].id.clone());
        let following = ConversionFollow::Preparing(7)
            .submitted(7, tasks[1].id.clone())
            .unwrap();
        // The old version remains readable but is not the explicit new intent.
        assert!(
            following
                .completed_task(true, 7, &draft, &tasks, &draft.input)
                .is_none()
        );
        tasks[1].state = workspace::TaskState::Complete;
        tasks[1].artifact = Some("versions/new-version".into());
        assert_eq!(
            following
                .completed_task(true, 7, &draft, &tasks, &draft.input)
                .unwrap()
                .id,
            "new-version",
        );
    }

    #[test]
    fn completion_opens_only_the_result_still_followed_in_the_current_input() {
        let (draft, mut tasks) = submitted_recovery_chain();
        let followed = ConversionFollow::Preparing(7)
            .submitted(7, "recovery".into())
            .unwrap();
        assert!(
            ConversionFollow::Preparing(7)
                .submitted(8, "recovery".into())
                .is_none()
        );
        assert!(
            followed
                .completed_task(true, 7, &draft, &tasks, &draft.input)
                .is_none()
        );
        tasks[1].state = workspace::TaskState::Complete;
        assert!(
            followed
                .completed_task(true, 7, &draft, &tasks, &draft.input)
                .is_none()
        );
        tasks[1].artifact = Some("versions/recovery".into());
        assert_eq!(
            followed
                .completed_task(true, 7, &draft, &tasks, &draft.input)
                .unwrap()
                .id,
            "recovery"
        );
        assert!(
            followed
                .completed_task(false, 7, &draft, &tasks, &draft.input)
                .is_none()
        );
        assert!(
            followed
                .completed_task(true, 8, &draft, &tasks, &draft.input)
                .is_none()
        );
        assert!(
            followed
                .completed_task(true, 7, &draft, &tasks, "new video")
                .is_none()
        );
        let other = ConversionFollow::Task {
            id: "original".into(),
            source_revision: 7,
        };
        assert!(
            other
                .completed_task(true, 7, &draft, &tasks, &draft.input)
                .is_none()
        );
        tasks[1].state = workspace::TaskState::Partial;
        assert!(
            followed
                .completed_task(true, 7, &draft, &tasks, &draft.input)
                .is_some()
        );
    }

    #[test]
    fn reader_repair_opens_the_new_version_only_while_the_original_note_is_still_open() {
        let (_, mut tasks) = submitted_recovery_chain();
        let original = std::path::Path::new("versions/original");
        let follow = ConversionFollow::reader_reprocess(&tasks[1], original, 12).unwrap();
        // Submitting a recovery is not enough: it must publish a readable result.
        assert!(!follow.completed_reader_task(&tasks[1], super::Page::Result, 12, Some(original)));
        tasks[1].artifact = Some("versions/recovery".into());
        for state in [
            workspace::TaskState::Complete,
            workspace::TaskState::Partial,
        ] {
            tasks[1].state = state;
            assert!(follow.completed_reader_task(
                &tasks[1],
                super::Page::Result,
                12,
                Some(original)
            ));
        }
        for page in [
            super::Page::New,
            super::Page::Library,
            super::Page::Task,
            super::Page::Settings,
        ] {
            assert!(!follow.completed_reader_task(&tasks[1], page, 12, Some(original)));
        }
        // Leaving and returning to the same note has a new read generation.
        assert!(!follow.completed_reader_task(&tasks[1], super::Page::Result, 13, Some(original)));
        assert!(!follow.completed_reader_task(
            &tasks[1],
            super::Page::Result,
            12,
            Some(std::path::Path::new("versions/another-note")),
        ));
        assert!(!follow.completed_reader_task(&tasks[1], super::Page::Result, 12, None));
        let mut unrelated = tasks[1].clone();
        unrelated.id = "another-repair".into();
        assert!(!follow.completed_reader_task(&unrelated, super::Page::Result, 12, Some(original)));
        for state in [
            workspace::TaskState::Running,
            workspace::TaskState::Paused,
            workspace::TaskState::Uncertain,
        ] {
            tasks[1].state = state;
            assert!(!follow.completed_reader_task(
                &tasks[1],
                super::Page::Result,
                12,
                Some(original)
            ));
        }
    }

    #[test]
    fn reader_repair_guard_excludes_exports_and_rechecks_the_note_when_loading_finishes() {
        let (_, mut tasks) = submitted_recovery_chain();
        let original = std::path::Path::new("versions/original");
        let follow = ConversionFollow::reader_reprocess(&tasks[1], original, 12).unwrap();
        assert!(follow.context_is_current(super::Page::Result, 99, 12, Some(original)));
        assert!(!follow.context_is_current(super::Page::Result, 99, 13, Some(original)));
        assert!(!follow.context_is_current(super::Page::Library, 99, 12, Some(original)));
        assert!(!follow.context_is_current(
            super::Page::Result,
            99,
            12,
            Some(std::path::Path::new("versions/other")),
        ));

        let course2md::execution::Operation::Reprocess { components, .. } =
            &mut tasks[1].plan.operation
        else {
            unreachable!();
        };
        *components = vec![course2md::execution::Component::Exports];
        tasks[1].state = workspace::TaskState::Complete;
        tasks[1].artifact = Some(original.to_owned());
        assert!(ConversionFollow::reader_reprocess(&tasks[1], original, 12).is_none());
        assert!(!follow.completed_reader_task(&tasks[1], super::Page::Result, 12, Some(original)));
        // Even a spurious new artifact cannot turn an export-only task into navigation.
        tasks[1].artifact = Some("versions/recovery".into());
        assert!(!follow.completed_reader_task(&tasks[1], super::Page::Result, 12, Some(original)));
        tasks[1].plan.operation = Default::default();
        assert!(ConversionFollow::reader_reprocess(&tasks[1], original, 12).is_none());
    }

    #[test]
    fn opening_a_reader_viewer_ends_repair_following_even_if_it_closes_before_the_read_finishes() {
        let (_, mut tasks) = submitted_recovery_chain();
        let original = std::path::Path::new("versions/original");
        let captured = ConversionFollow::reader_reprocess(&tasks[1], original, 12).unwrap();
        tasks[1].state = workspace::TaskState::Complete;
        tasks[1].artifact = Some("versions/recovery".into());
        for read_started in [false, true] {
            let mut active = Some(captured.clone());
            if read_started {
                assert!(active.as_ref().unwrap().completed_reader_task(
                    &tasks[1],
                    super::Page::Result,
                    12,
                    Some(original),
                ));
            }
            ConversionFollow::interrupt_for_reader_viewer(&mut active);
            assert!(active.is_none());
            // Closing the viewer only restores its underlying note. The old
            // asynchronous callback still has a ticket, but no longer owns intent.
            assert_ne!(active.as_ref(), Some(&captured));
            assert!(
                !active
                    .as_ref()
                    .is_some_and(|follow| follow.completed_reader_task(
                        &tasks[1],
                        super::Page::Result,
                        12,
                        Some(original),
                    ))
            );
        }
        // A viewer must not cancel a distinct workbench conversion.
        let mut input = Some(ConversionFollow::Task {
            id: "conversion".into(),
            source_revision: 7,
        });
        let expected = input.clone();
        ConversionFollow::interrupt_for_reader_viewer(&mut input);
        assert_eq!(input, expected);
    }

    #[test]
    fn completion_belongs_to_the_submitted_input_and_requires_a_readable_result() {
        let root = tempfile::tempdir().unwrap();
        let mut workspace = workspace::Workspace::open_at(
            root.path().join("workspace.json"),
            root.path().join("library"),
            Default::default(),
        )
        .unwrap();
        let state = &mut workspace.state;
        let source = source::Source {
            input: "https://www.bilibili.com/video/BVfixture".into(),
            identity: "video-fixture".into(),
            online: true,
            ..Default::default()
        };
        let plan = workspace::TaskPlan {
            operation: Default::default(),
            source: source.clone(),
            source_id: source.identity.clone(),
            title: "已提交的标题".into(),
            library_id: state.default_library.clone(),
            folder: None,
            options: Default::default(),
            subtitle: None,
            config: Default::default(),
            asr_service: None,
            ai_service: None,
            study: Default::default(),
        };
        let (id, _) = state.enqueue(plan.clone(), None).unwrap();
        let draft = state.draft_mut().unwrap();
        draft.change_source(source.input.clone());
        draft.submitted_task = Some(id.clone());
        assert!(
            completed_input_task(state.draft().unwrap(), &state.tasks, &source.input).is_none()
        );
        state.task_mut(&id).unwrap().state = workspace::TaskState::Complete;
        assert!(
            completed_input_task(state.draft().unwrap(), &state.tasks, &source.input).is_none()
        );
        state.task_mut(&id).unwrap().artifact = Some(root.path().join("published-version"));
        assert_eq!(
            completed_input_task(state.draft().unwrap(), &state.tasks, &source.input)
                .unwrap()
                .id,
            id,
        );
        // The text can change before its observer replaces the submitted draft.
        // That next input must remain visible even during this intermediate state.
        for next in ["", "https://www.bilibili.com/video/BVnext"] {
            assert!(completed_input_task(state.draft().unwrap(), &state.tasks, next).is_none());
        }
        state.task_mut(&id).unwrap().state = workspace::TaskState::Partial;
        assert!(
            completed_input_task(state.draft().unwrap(), &state.tasks, &source.input).is_some()
        );
        state.task_mut(&id).unwrap().state = workspace::TaskState::Complete;
        state.prepare_next_import("https://www.bilibili.com/video/BVnext", Default::default());
        assert!(
            completed_input_task(state.draft().unwrap(), &state.tasks, &source.input).is_none()
        );
        assert!(state.task(&id).unwrap().plan == plan);
    }

    fn submitted_recovery_chain() -> (workspace::Draft, Vec<workspace::TaskRecord>) {
        let source = source::Source {
            input: "https://www.bilibili.com/video/BVfixture".into(),
            identity: "video-fixture".into(),
            online: true,
            ..Default::default()
        };
        let mut draft = workspace::Draft::new(true, "library".into(), Default::default());
        draft.change_source(source.input.clone());
        draft.source = Some(source.clone());
        draft.submitted_task = Some("original".into());
        let mut original = workspace::TaskRecord {
            id: "original".into(),
            plan: workspace::TaskPlan {
                operation: Default::default(),
                source_id: source.identity.clone(),
                source,
                title: "已提交的课程".into(),
                library_id: "library".into(),
                folder: None,
                options: Default::default(),
                subtitle: None,
                config: Default::default(),
                asr_service: None,
                ai_service: None,
                study: Default::default(),
            },
            state: workspace::TaskState::Uncertain,
            intent: workspace::Intent::Pause,
            created: 0,
            updated: 0,
            parent: None,
            handled_by: None,
            work_dir: "work/original".into(),
            stages: Default::default(),
            error: Some("原摘要请求结果未确认".into()),
            artifact: Some("versions/original".into()),
            outcomes: None,
            unread: false,
            logs: Vec::new(),
            blocked: Vec::new(),
            resend: Vec::new(),
            study: Default::default(),
            study_meter: None,
        };
        let mut recovery = original.clone();
        recovery.id = "recovery".into();
        recovery.parent = Some(original.id.clone());
        recovery.state = workspace::TaskState::Queued;
        recovery.intent = workspace::Intent::Run;
        recovery.error = None;
        recovery.artifact = None;
        recovery.work_dir = "work/recovery".into();
        recovery.plan.operation = course2md::execution::Operation::Reprocess {
            base_version_dir: "versions/original".into(),
            components: vec![course2md::execution::Component::Summary],
            prior_work_dir: Some(original.work_dir.clone()),
        };
        original.handled_by = Some(recovery.id.clone());
        (draft, vec![original, recovery])
    }

    #[test]
    fn workbench_entry_retires_only_unchanged_terminal_inputs() {
        let (mut draft, mut tasks) = submitted_recovery_chain();
        draft.title = tasks[0].plan.title.clone();
        for state in [
            workspace::TaskState::Complete,
            workspace::TaskState::Partial,
            workspace::TaskState::Cancelled,
        ] {
            tasks[1].state = state;
            tasks[1].artifact = Some("versions/recovery".into());
            assert_eq!(
                super::retired_input_task(&draft, &tasks, &draft.input)
                    .unwrap()
                    .id,
                "recovery"
            );
        }
        for state in [
            workspace::TaskState::Queued,
            workspace::TaskState::Running,
            workspace::TaskState::Pausing,
            workspace::TaskState::Paused,
            workspace::TaskState::NeedsAttention,
            workspace::TaskState::Uncertain,
        ] {
            tasks[1].state = state;
            assert!(super::retired_input_task(&draft, &tasks, &draft.input).is_none());
        }
        tasks[1].state = workspace::TaskState::Partial;
        let unchanged = draft.clone();
        draft.title = "尚未提交的新名称".into();
        assert!(super::retired_input_task(&draft, &tasks, &draft.input).is_none());
        draft = unchanged.clone();
        draft.options.vision = !draft.options.vision;
        assert!(super::retired_input_task(&draft, &tasks, &draft.input).is_none());
        draft = unchanged.clone();
        draft.folder = Some(42);
        assert!(super::retired_input_task(&draft, &tasks, &draft.input).is_none());
        draft = unchanged.clone();
        draft.ai_model = Some("changed-after-submission".into());
        assert!(super::retired_input_task(&draft, &tasks, &draft.input).is_none());
        draft = unchanged;
        assert!(super::retired_input_task(&draft, &tasks, "新视频").is_none());
        tasks[1].blocked.push(workspace::BlockedRequest {
            reason: course2md::dispatch::REASON_UNCERTAIN.into(),
            request_id: Some("pending-summary".into()),
            purpose: Some("summary".into()),
            description: "摘要".into(),
            kind: Some(course2md::artifact::FailureKind::Uncertain),
            message: "结果未知".into(),
        });
        assert!(super::retired_input_task(&draft, &tasks, &draft.input).is_none());
        tasks[1].blocked.clear();
        tasks[1].artifact = None;
        assert!(super::retired_input_task(&draft, &tasks, &draft.input).is_none());
    }

    #[test]
    fn workbench_follows_the_same_recovery_for_queued_running_and_completed_results() {
        let (draft, mut tasks) = submitted_recovery_chain();
        let original = tasks[0].clone();
        for state in [workspace::TaskState::Queued, workspace::TaskState::Running] {
            tasks[1].state = state;
            assert_eq!(
                submitted_input_task(&draft, &tasks, &draft.input)
                    .unwrap()
                    .id,
                "recovery"
            );
            assert!(completed_input_task(&draft, &tasks, &draft.input).is_none());
        }
        tasks[1].state = workspace::TaskState::Complete;
        assert!(completed_input_task(&draft, &tasks, &draft.input).is_none());
        tasks[1].artifact = Some("versions/recovery".into());
        assert_eq!(
            completed_input_task(&draft, &tasks, &draft.input)
                .unwrap()
                .id,
            "recovery"
        );

        let mut final_attempt = tasks[1].clone();
        final_attempt.id = "final-attempt".into();
        final_attempt.parent = Some("recovery".into());
        final_attempt.state = workspace::TaskState::Running;
        final_attempt.artifact = None;
        tasks[1].state = workspace::TaskState::Uncertain;
        tasks[1].handled_by = Some(final_attempt.id.clone());
        tasks.push(final_attempt);
        assert_eq!(
            submitted_input_task(&draft, &tasks, &draft.input)
                .unwrap()
                .id,
            "final-attempt"
        );
        assert!(completed_input_task(&draft, &tasks, &draft.input).is_none());
        tasks[2].state = workspace::TaskState::Complete;
        tasks[2].artifact = Some("versions/final-attempt".into());
        assert_eq!(
            completed_input_task(&draft, &tasks, &draft.input)
                .unwrap()
                .id,
            "final-attempt"
        );
        // A cancelled recovery still belongs to this input for inline feedback,
        // but must not fall back to an older readable result or follow new input.
        tasks[2].state = workspace::TaskState::Cancelled;
        tasks[2].artifact = None;
        assert_eq!(
            submitted_input_task(&draft, &tasks, &draft.input)
                .unwrap()
                .id,
            "final-attempt"
        );
        assert!(completed_input_task(&draft, &tasks, &draft.input).is_none());
        assert!(submitted_input_task(&draft, &tasks, "another-video").is_none());
        assert!(tasks[0] == original);
        assert_eq!(draft.submitted_task.as_deref(), Some("original"));
    }

    #[test]
    fn recovery_lookup_rejects_broken_cycles_or_unrelated_task_links() {
        for case in [
            "missing",
            "self-cycle",
            "cycle",
            "wrong-parent",
            "other-source",
            "inconsistent-source",
            "other-kind",
            "duplicate-id",
            "unrelated-submission",
        ] {
            let (mut draft, mut tasks) = submitted_recovery_chain();
            match case {
                "missing" => tasks[0].handled_by = Some("missing".into()),
                "self-cycle" => {
                    tasks[0].handled_by = Some("original".into());
                    tasks[0].parent = Some("original".into());
                }
                "cycle" => {
                    tasks[1].handled_by = Some("original".into());
                    tasks[0].parent = Some("recovery".into());
                }
                "wrong-parent" => tasks[1].parent = Some("unrelated".into()),
                "other-source" => {
                    tasks[1].plan.source_id = "other-video".into();
                    tasks[1].plan.source.identity = "other-video".into();
                }
                "inconsistent-source" => tasks[1].plan.source.identity = "other-video".into(),
                "other-kind" => tasks[1].plan.source.online = false,
                "duplicate-id" => tasks.push(tasks[1].clone()),
                "unrelated-submission" => {
                    tasks[1].plan.source_id = "other-video".into();
                    tasks[1].plan.source.identity = "other-video".into();
                    draft.submitted_task = Some("recovery".into());
                }
                _ => unreachable!(),
            }
            assert!(
                submitted_input_task(&draft, &tasks, &draft.input).is_none(),
                "{case}"
            );
            assert!(
                completed_input_task(&draft, &tasks, &draft.input).is_none(),
                "{case}"
            );
        }
    }

    #[test]
    fn a_recovered_result_cannot_replace_new_or_changed_input() {
        let (mut draft, mut tasks) = submitted_recovery_chain();
        tasks[1].state = workspace::TaskState::Complete;
        tasks[1].artifact = Some("versions/recovery".into());
        for input in ["", "https://www.bilibili.com/video/BVnext"] {
            assert!(submitted_input_task(&draft, &tasks, input).is_none());
            assert!(completed_input_task(&draft, &tasks, input).is_none());
        }
        draft.source.as_mut().unwrap().identity = "newly-inspected-content".into();
        assert!(submitted_input_task(&draft, &tasks, &draft.input).is_none());
        draft.source = None;
        draft.input = "https://www.bilibili.com/video/BVnext".into();
        assert!(submitted_input_task(&draft, &tasks, &draft.input).is_none());
    }

    #[test]
    fn automatic_mode_uses_speech_after_login_or_download_failure() {
        let track = course2md::subtitle::file_track("lesson.srt".into(), None);
        for metadata_failed in [true, false] {
            let mut source = source::Source {
                subtitles: if metadata_failed {
                    SubtitleEvidence::Failed {
                        message: "WARNING: subtitles require login".into(),
                    }
                } else {
                    SubtitleEvidence::Found {
                        tracks: vec![track.clone()],
                        warning: None,
                    }
                },
                subtitle_request: (!metadata_failed).then(|| track.clone()),
                subtitle_read_error: (!metadata_failed).then(|| SubtitleReadError::Failed {
                    message: "字幕文件没有下载成功".into(),
                }),
                ..Default::default()
            };
            assert!(automatic_subtitle_fallback(
                &mut source,
                TranscriptSource::Auto,
                None
            ));
            assert!(uses_speech(&source, TranscriptSource::Auto));
            assert!(!subtitle_needs_confirmation(
                &source,
                TranscriptSource::Auto
            ));
            assert!(source.subtitle_request.is_none());
            assert!(source.subtitle_read_error.is_none());
        }
    }

    #[test]
    fn explicit_subtitle_and_cancelled_reads_never_fall_back_to_speech() {
        let mut source = source::Source {
            subtitles: SubtitleEvidence::Failed {
                message: "字幕需要登录".into(),
            },
            subtitle_read_error: Some(SubtitleReadError::Failed {
                message: "字幕需要登录".into(),
            }),
            ..Default::default()
        };
        let original = source.clone();
        assert!(!automatic_subtitle_fallback(
            &mut source,
            TranscriptSource::Subtitle,
            None
        ));
        assert_eq!(source, original);
        assert!(subtitle_needs_confirmation(
            &source,
            TranscriptSource::Subtitle
        ));

        source.subtitle_read_error = Some(SubtitleReadError::Cancelled);
        let cancelled = source.clone();
        assert!(!automatic_subtitle_fallback(
            &mut source,
            TranscriptSource::Auto,
            None
        ));
        assert_eq!(source, cancelled);
        assert!(!uses_speech(&source, TranscriptSource::Auto));
    }

    #[test]
    fn pending_or_failed_subtitles_remain_required_even_with_cached_text() {
        let track = course2md::subtitle::file_track("lesson.srt".into(), None);
        let mut source = source::Source {
            identity: "lesson".into(),
            subtitles: SubtitleEvidence::Found {
                tracks: vec![track.clone()],
                warning: None,
            },
            ..Default::default()
        };
        assert!(subtitle_needs_confirmation(&source, TranscriptSource::Auto));
        source.selected_subtitle = Some(CachedSubtitle {
            source_identity: source.identity.clone(),
            track_id: track.id.clone(),
            label: "已读取字幕".into(),
            path: "lesson.srt".into(),
            events: Vec::new(),
        });
        assert!(!subtitle_needs_confirmation(
            &source,
            TranscriptSource::Auto
        ));

        source.subtitle_request = Some(track);
        assert!(subtitle_needs_confirmation(&source, TranscriptSource::Auto));
        source.subtitle_request = None;
        source.subtitle_read_error = Some(SubtitleReadError::Failed {
            message: "无法读取新字幕，原正文仍保留".into(),
        });
        assert!(subtitle_needs_confirmation(&source, TranscriptSource::Auto));

        // Cancelling the replacement or choosing the cached text restores the
        // confirmed source without opening unrelated generation preferences.
        source.subtitle_read_error = None;
        assert!(!subtitle_needs_confirmation(
            &source,
            TranscriptSource::Subtitle
        ));
        source.selected_subtitle = None;
        assert!(subtitle_needs_confirmation(
            &source,
            TranscriptSource::Subtitle
        ));
        assert!(!subtitle_needs_confirmation(&source, TranscriptSource::Asr));
    }

    #[test]
    fn caption_intent_and_unconfirmed_reads_never_imply_speech_recognition() {
        let mut source = source::Source {
            subtitles: SubtitleEvidence::NoneFound,
            ..Default::default()
        };
        assert!(uses_speech(&source, TranscriptSource::Auto));
        assert!(!uses_speech(&source, TranscriptSource::Subtitle));
        source.subtitle_request = Some(course2md::subtitle::file_track("lesson.srt".into(), None));
        assert!(!uses_speech(&source, TranscriptSource::Auto));
        assert!(!uses_speech(&source, TranscriptSource::Subtitle));
        source.subtitle_request = None;
        source.subtitle_read_error = Some(SubtitleReadError::Failed {
            message: "字幕文件无法读取".into(),
        });
        assert!(!uses_speech(&source, TranscriptSource::Auto));
        assert!(uses_speech(&source, TranscriptSource::Asr));
        source.subtitle_read_error = None;
        source.subtitles = SubtitleEvidence::Unchecked;
        assert!(!uses_speech(&source, TranscriptSource::Auto));
        source.subtitles = SubtitleEvidence::Failed {
            message: "请求失败".into(),
        };
        assert!(uses_speech(&source, TranscriptSource::Auto));
        assert!(!uses_speech(&source, TranscriptSource::Subtitle));
    }
}
