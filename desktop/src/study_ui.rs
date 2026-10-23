//! Explicit bridge to the project's image-and-transcript study-note generator.
//! File inspection and child processes only run on the blocking IO worker.
use super::*;
use crate::preferences::{ServicePurpose, ServiceVersion};
use crate::theme::*;
use gpui_component::{
    button::ButtonVariants,
    dialog::Dialog,
    input::{Textarea, TextareaState},
    menu::{DropdownMenu, PopupMenuItem},
};
use std::io::Write as _;
use std::{
    ffi::OsString,
    path::Path,
    process::{Command, Stdio},
};

fn help(content: impl Into<SharedString>) -> Div {
    div()
        .min_w_0()
        .whitespace_normal()
        .text_size(TEXT_BODY)
        .line_height(relative(1.5))
        .text_color(color(GRAY))
        .child(content.into())
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum Phase {
    #[default]
    Idle,
    Inspecting,
    Ready,
    Running,
    Complete,
    Failed,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum Mode {
    #[default]
    Native,
    ChatGptExport,
}
impl Phase {
    fn busy(self) -> bool {
        matches!(self, Self::Inspecting | Self::Running)
    }
    fn can_start(self) -> bool {
        self == Self::Ready
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum Limit {
    #[default]
    Three,
    Ten,
    All,
}
impl Limit {
    fn id(self) -> &'static str {
        match self {
            Self::Three => "3",
            Self::Ten => "10",
            Self::All => "all",
        }
    }
    fn from_id(id: &str) -> Option<Self> {
        match id {
            "3" => Some(Self::Three),
            "10" => Some(Self::Ten),
            "all" => Some(Self::All),
            _ => None,
        }
    }
}

#[derive(Clone)]
struct Tool {
    python: PathBuf,
    script: PathBuf,
}
#[derive(Clone, PartialEq, Eq, serde::Deserialize)]
struct Description {
    unit_count: usize,
    outputs_root: PathBuf,
    prompt_sha256: String,
}
#[derive(serde::Deserialize)]
struct ResultFile {
    status: String,
    #[serde(default)]
    output_html: Option<PathBuf>,
    #[serde(default)]
    error: Option<String>,
    #[serde(default)]
    export_zip: Option<PathBuf>,
}

#[derive(Default)]
pub(crate) struct State {
    course: PathBuf,
    title: String,
    tool: Option<Tool>,
    description: Option<Description>,
    phase: Phase,
    meter: Option<crate::study_progress::Meter>,
    limit: Limit,
    output: Option<PathBuf>,
    error: Option<String>,
    open_status: Option<(String, bool)>,
    revision: u64,
    mode: Mode,
    service: Option<String>,
    model: Option<Entity<InputState>>,
    prompt: Option<Entity<TextareaState>>,
    standard_prompt: String,
    show_prompt: bool,
    prompt_status: Option<String>,
    models: crate::model_discovery::State,
    _model_subscription: Option<Subscription>,
    _prompt_subscription: Option<Subscription>,
    export_zip: Option<PathBuf>,
    catalog_generation: u64,
}

/// Secrets are resolved only by the worker and travel through the child pipe.
/// This type deliberately cannot be serialized, persisted, or debug-printed.
struct NativeRequest {
    settings: course2md::llm::LlmSettings,
    credential: Option<crate::credentials::CredentialRef>,
    vault: std::sync::Arc<dyn crate::credentials::CredentialVault>,
}

type Generated = anyhow::Result<(Phase, Option<PathBuf>, Option<String>, Option<PathBuf>)>;
enum StudyEvent {
    Progress(crate::study_progress::Meter),
    Finished(Generated),
}

/// One ordered stream ensures the final result cannot overtake its progress.
fn study_worker(
    work: impl FnOnce(smol::channel::Sender<StudyEvent>) -> Generated + Send + 'static,
) -> smol::channel::Receiver<StudyEvent> {
    let (tx, rx) = smol::channel::bounded(16);
    std::thread::spawn(move || {
        let result = work(tx.clone());
        let _ = tx.send_blocking(StudyEvent::Finished(result));
    });
    rx
}

#[derive(serde::Serialize)]
struct StudyPayload<'a> {
    settings: &'a course2md::llm::LlmSettings,
    prompt: Option<&'a str>,
}

fn request_args(script: &Path, course: &Path, limit: Limit, result: &Path) -> Vec<OsString> {
    vec![
        "-X".into(),
        "utf8".into(),
        "-B".into(),
        script.as_os_str().into(),
        "--course".into(),
        course.as_os_str().into(),
        "--limit".into(),
        limit.id().into(),
        "--result-file".into(),
        result.as_os_str().into(),
    ]
}

fn hidden_command(python: &Path) -> Command {
    let mut command = Command::new(python);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x0800_0000);
    }
    command.stdin(Stdio::null()).stderr(Stdio::null());
    command
}

fn configured_tool() -> anyhow::Result<Tool> {
    let python = std::env::var_os("COURSE2MD_STUDY_PYTHON").map(PathBuf::from);
    let script = std::env::var_os("COURSE2MD_STUDY_SCRIPT").map(PathBuf::from);
    let (Some(python), Some(script)) = (python, script) else {
        anyhow::bail!(course2md::tr!(
            "未找到学习讲义工具，请从项目统一入口启动课程软件。"
        ));
    };
    anyhow::ensure!(
        python.is_file() && script.is_file(),
        course2md::tr!("学习讲义工具路径已失效，请从项目统一入口启动。")
    );
    Ok(Tool { python, script })
}

fn standard_prompt_from_script(script: &Path) -> anyhow::Result<String> {
    let template_dir = script.parent().ok_or_else(|| {
        anyhow::anyhow!(course2md::tr!("找不到标准教学提示词，请检查学习讲义工具。"))
    })?;
    let prompt = std::fs::read_to_string(template_dir.join("task.txt"))?
        + "\n"
        + &std::fs::read_to_string(template_dir.join("prompt43.txt"))?;
    anyhow::ensure!(
        !prompt.trim().is_empty() && prompt.len() <= 131_072,
        course2md::tr!("标准教学提示词为空或过长，请检查学习讲义工具。")
    );
    Ok(prompt)
}

pub(crate) fn standard_study_prompt() -> anyhow::Result<String> {
    standard_prompt_from_script(&configured_tool()?.script)
}

pub(crate) fn study_tool_ready() -> anyhow::Result<()> {
    let _ = standard_study_prompt()?;
    Ok(())
}

fn inspect(course: &Path) -> anyhow::Result<(Tool, Description, String)> {
    let tool = configured_tool()?;
    let stdout = tempfile::NamedTempFile::new()?;
    let mut child = hidden_command(&tool.python)
        .args([
            OsString::from("-X"),
            "utf8".into(),
            "-B".into(),
            tool.script.as_os_str().into(),
            "--describe".into(),
            "--course".into(),
            course.as_os_str().into(),
        ])
        .stdout(stdout.reopen()?)
        .spawn()?;
    let started = Instant::now();
    let status = loop {
        if let Some(status) = child.try_wait()? {
            break status;
        }
        if started.elapsed() > Duration::from_secs(30) {
            let _ = child.kill();
            let _ = child.wait();
            anyhow::bail!(course2md::tr!("读取本地讲义配置超时，请关闭后重试。"));
        }
        std::thread::sleep(Duration::from_millis(50));
    };
    anyhow::ensure!(
        status.success(),
        course2md::tr!("无法读取讲义配置；请检查工具的配置和课程资料。")
    );
    anyhow::ensure!(
        stdout.as_file().metadata()?.len() <= 1_048_576,
        course2md::tr!("讲义配置不完整，请检查模型和 43 号提示词文件。")
    );
    let description: Description = serde_json::from_slice(&std::fs::read(stdout.path())?)?;
    anyhow::ensure!(
        description.prompt_sha256.len() == 64
            && description
                .prompt_sha256
                .bytes()
                .all(|byte| byte.is_ascii_hexdigit()),
        course2md::tr!("讲义配置不完整，请检查模型和 43 号提示词文件。")
    );
    let prompt = standard_prompt_from_script(&tool.script)?;
    Ok((tool, description, prompt))
}

fn generate(
    tool: Tool,
    course: PathBuf,
    limit: Limit,
    expected: Description,
    native: Option<NativeRequest>,
    prompt: Option<String>,
    progress: smol::channel::Sender<StudyEvent>,
) -> anyhow::Result<(Phase, Option<PathBuf>, Option<String>, Option<PathBuf>)> {
    // Reinspect locally before dispatch: never silently use a model changed after confirmation.
    let (current_tool, current, _) = inspect(&course)?;
    anyhow::ensure!(
        current == expected
            && current_tool.python == tool.python
            && current_tool.script == tool.script,
        course2md::tr!("讲义配置或课程资料已改变；请重新打开并确认后生成。")
    );
    let temporary = tempfile::tempdir()?;
    let result_path = temporary.path().join("study-result.json");
    let progress_path = temporary.path().join("study-progress.json");
    let mut command = hidden_command(&tool.python);
    command
        .args(request_args(&tool.script, &course, limit, &result_path))
        .arg("--progress-file")
        .arg(&progress_path)
        .stdout(Stdio::null());
    let manual = native.is_none();
    let mut settings = if let Some(mut native) = native {
        if let Some(reference) = &native.credential {
            native.settings.api_key = native.vault.resolve(reference)?.expose().to_owned();
        }
        if native.settings.provider == course2md::llm::LlmProvider::Codex {
            anyhow::ensure!(
                course2md::login::codex::status()?
                    == course2md::login::codex::LoginStatus::Connected,
                course2md::tr!("请先连接 ChatGPT 订阅账号，再生成讲义。")
            );
        }
        course2md::llm::validate(&native.settings)?;
        native.settings
    } else {
        command.arg("--export-chatgpt");
        course2md::llm::LlmSettings::default()
    };
    command.arg("--native-ai-stdin").stdin(Stdio::piped());
    let payload = zeroize::Zeroizing::new(serde_json::to_vec(&StudyPayload {
        settings: &settings,
        prompt: prompt.as_deref(),
    })?);
    use zeroize::Zeroize as _;
    settings.api_key.zeroize();
    let mut child = command.spawn()?;
    let written = child
        .stdin
        .take()
        .ok_or_else(|| anyhow::anyhow!("missing child input"))?
        .write_all(&payload);
    if let Err(error) = written {
        let _ = child.kill();
        let _ = child.wait();
        return Err(error.into());
    }
    let mut latest = crate::study_progress::Meter::default();
    let exit = loop {
        let exit = child.try_wait()?;
        if let Some(meter) = crate::study_progress::read(&progress_path)
            && latest != meter
            && latest.accepts(&meter)
        {
            latest = meter.clone();
            let _ = progress.send_blocking(StudyEvent::Progress(meter));
        }
        if let Some(exit) = exit {
            break exit;
        }
        std::thread::sleep(Duration::from_millis(200));
    };
    let result: ResultFile =
        serde_json::from_slice(&std::fs::read(result_path).map_err(|_| {
            anyhow::anyhow!(course2md::tr!(
                "讲义工具未返回结果文件，请查看项目运行日志。"
            ))
        })?)?;
    let output = result
        .output_html
        .filter(|path| path.is_absolute() && path.is_file());
    let export_zip = result
        .export_zip
        .filter(|path| path.is_absolute() && path.is_file());
    if result_phase(
        &result.status,
        exit.success(),
        output.is_some() && (!manual || export_zip.is_some()),
    ) == Phase::Complete
    {
        Ok((Phase::Complete, output, None, export_zip))
    } else {
        Ok((
            Phase::Failed,
            output,
            Some(
                result
                    .error
                    .filter(|message| !message.is_empty())
                    .unwrap_or_else(|| {
                        course2md::tr!("部分讲义未生成，请查看已生成内容和项目运行日志。").into()
                    }),
            ),
            export_zip,
        ))
    }
}

fn result_phase(status: &str, process_succeeded: bool, has_readable_output: bool) -> Phase {
    if status == "success" && process_succeeded && has_readable_output {
        Phase::Complete
    } else {
        Phase::Failed
    }
}

fn study_launchable(
    task_state: crate::workspace::TaskState,
    progress: &crate::workspace::StudyProgress,
    enabled: bool,
) -> bool {
    enabled
        && matches!(
            task_state,
            crate::workspace::TaskState::Complete | crate::workspace::TaskState::Partial
        )
        && matches!(progress, crate::workspace::StudyProgress::Pending)
}

struct StudyDialog {
    desktop: Entity<Desktop>,
    footer: bool,
    _observation: Subscription,
}
impl Render for StudyDialog {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.desktop.update(cx, |desktop, cx| {
            if self.footer {
                desktop.study_dialog_actions(cx).into_any_element()
            } else {
                desktop
                    .study_dialog_content(cx)
                    .id("study-dialog-scroll")
                    .max_h((window.bounds().size.height - px(310.)).max(px(100.)))
                    .overflow_y_scroll()
                    .into_any_element()
            }
        })
    }
}

impl Desktop {
    pub(crate) fn set_study_open_status(
        &mut self,
        revision: u64,
        path: &Path,
        message: String,
        failed: bool,
    ) {
        if self.study_ui.revision == revision && self.study_ui.output.as_deref() == Some(path) {
            self.study_ui.open_status = Some((message, failed));
        }
    }

    /// Continue an explicitly submitted conversion with its frozen teaching
    /// settings. The raw note has already been published before this is called.
    pub(crate) fn start_automatic_study(
        &mut self,
        task_id: String,
        course: Course,
        foreground: bool,
        cx: &mut Context<Self>,
    ) {
        if self.closing || self.storage_ui.busy {
            return;
        }
        let Some(task) = self
            .workspace
            .as_ref()
            .and_then(|workspace| workspace.state.task(&task_id))
            .cloned()
        else {
            return;
        };
        if !study_launchable(task.state, &task.study, task.plan.study.enabled) {
            return;
        }
        let selection = task.plan.study.clone();
        let native = match Self::native_request_for_selection(
            &self.preferences,
            selection.service.as_deref(),
            selection.model.as_deref().unwrap_or_default(),
        ) {
            Ok(request) => request,
            Err(error) => {
                self.finish_automatic_study(
                    &task_id,
                    crate::workspace::StudyProgress::Failed {
                        error: format!("{error:#}"),
                    },
                    foreground,
                    Some(course),
                    cx,
                );
                return;
            }
        };
        let Some(workspace) = &mut self.workspace else {
            return;
        };
        if let Err(error) = workspace.transaction(|state| {
            let record = state
                .task_mut(&task_id)
                .ok_or_else(|| anyhow::anyhow!("missing task"))?;
            anyhow::ensure!(
                study_launchable(record.state, &record.study, record.plan.study.enabled),
                "study request already started"
            );
            record.study = crate::workspace::StudyProgress::Running;
            record.study_meter = Some(Default::default());
            record.updated = crate::workspace::now();
            Ok(())
        }) {
            self.workspace_error = Some(format!("{error:#}"));
            cx.notify();
            return;
        }
        let course_path = course.dir.clone();
        let prompt = selection.prompt;
        let work = study_worker(move |progress| {
            let (tool, description, standard) = inspect(&course_path)?;
            anyhow::ensure!(
                description.unit_count > 0,
                course2md::tr!("本课程没有可处理的截图与字幕，请先完成转录。")
            );
            let override_prompt = (prompt != standard).then_some(prompt);
            generate(
                tool,
                course_path,
                Limit::All,
                description,
                Some(native),
                override_prompt,
                progress,
            )
        });
        let follow = self.following_conversion.clone();
        cx.spawn(async move |this, cx| {
            let result = loop {
                match work.recv().await {
                    Ok(StudyEvent::Progress(meter)) => {
                        let _ = this.update(cx, |this, cx| {
                            let Some(workspace) = &mut this.workspace else {
                                return;
                            };
                            let current = workspace.state.task(&task_id).is_some_and(|task| {
                                task.study == crate::workspace::StudyProgress::Running
                                    && task
                                        .study_meter
                                        .as_ref()
                                        .is_none_or(|previous| previous.accepts(&meter))
                            });
                            if !current {
                                return;
                            }
                            if let Err(error) = workspace.transaction(|state| {
                                if let Some(task) = state.task_mut(&task_id) {
                                    task.study_meter = Some(meter);
                                }
                                Ok(())
                            }) {
                                this.workspace_error = Some(error.to_string());
                            }
                            cx.notify();
                        });
                    }
                    Ok(StudyEvent::Finished(result)) => break Ok(result),
                    Err(error) => break Err(error),
                }
            };
            let _ = this.update(cx, |this, cx| {
                let still_foreground = foreground
                    && !this.reader_viewer_open()
                    && ((follow.is_some() && this.following_conversion == follow)
                        || (this.page == Page::Task
                            && this.workspace.as_ref().is_some_and(|workspace| {
                                workspace.state.selected_task.as_deref() == Some(task_id.as_str())
                            })));
                let progress = match result {
                    Ok(Ok((Phase::Complete, Some(output), _, _))) => {
                        crate::workspace::StudyProgress::Complete { output }
                    }
                    Ok(Ok((_, _, error, _))) => crate::workspace::StudyProgress::Failed {
                        error: error.unwrap_or_else(|| {
                            course2md::tr!("部分讲义未生成，请查看已生成内容和项目运行日志。")
                                .into()
                        }),
                    },
                    Ok(Err(error)) => crate::workspace::StudyProgress::Uncertain {
                        error: format!("{error:#}"),
                    },
                    Err(_) => crate::workspace::StudyProgress::Uncertain {
                        error: course2md::tr!("操作意外中断，请重试").into(),
                    },
                };
                this.finish_automatic_study(&task_id, progress, still_foreground, Some(course), cx);
            });
        })
        .detach();
        cx.notify();
    }

    fn finish_automatic_study(
        &mut self,
        task_id: &str,
        progress: crate::workspace::StudyProgress,
        foreground: bool,
        course: Option<Course>,
        cx: &mut Context<Self>,
    ) {
        let Some(workspace) = &mut self.workspace else {
            return;
        };
        if let Err(error) = workspace.transaction(|state| {
            let task = state
                .task_mut(task_id)
                .ok_or_else(|| anyhow::anyhow!("missing task"))?;
            task.study = progress.clone();
            task.updated = crate::workspace::now();
            task.unread = !foreground;
            Ok(())
        }) {
            self.workspace_error = Some(format!("{error:#}"));
            cx.notify();
            return;
        }
        if foreground {
            match progress {
                crate::workspace::StudyProgress::Complete { output } => {
                    self.open_study_document(output, cx);
                }
                crate::workspace::StudyProgress::Failed { error }
                | crate::workspace::StudyProgress::Uncertain { error } => {
                    self.message = Some(error);
                    if let Some(course) = course {
                        self.open_completed_conversion(course, cx);
                    }
                }
                _ => {}
            }
        }
        self.resume_pending_studies(cx);
        cx.notify();
    }

    /// A retry is only reached from an explicit task action. In particular an
    /// uncertain remote request is not resumed during workspace recovery.
    pub(crate) fn retry_automatic_study(&mut self, task_id: &str, cx: &mut Context<Self>) {
        let Some(task) = self
            .workspace
            .as_ref()
            .and_then(|workspace| workspace.state.task(task_id))
            .cloned()
        else {
            return;
        };
        if !task.plan.study.enabled
            || !matches!(
                task.state,
                crate::workspace::TaskState::Complete | crate::workspace::TaskState::Partial
            )
            || !matches!(
                task.study,
                crate::workspace::StudyProgress::Failed { .. }
                    | crate::workspace::StudyProgress::Uncertain { .. }
            )
        {
            return;
        }
        let Some(path) = task.artifact.clone() else {
            return;
        };
        if let Some(workspace) = &mut self.workspace
            && let Err(error) = workspace.transaction(|state| {
                let record = state
                    .task_mut(task_id)
                    .ok_or_else(|| anyhow::anyhow!("missing task"))?;
                record.study = crate::workspace::StudyProgress::Pending;
                record.study_meter = None;
                record.updated = crate::workspace::now();
                Ok(())
            })
        {
            self.workspace_error = Some(format!("{error:#}"));
            cx.notify();
            return;
        }
        let course = Course::from_completed(&crate::backend::Completed {
            out_dir: path,
            title: task.plan.title,
            ..Default::default()
        });
        self.start_automatic_study(task_id.to_owned(), course, true, cx);
    }

    pub(crate) fn resume_pending_studies(&mut self, cx: &mut Context<Self>) {
        if self.storage_ui.busy {
            return;
        }
        let Some(workspace) = &self.workspace else {
            return;
        };
        if workspace
            .state
            .tasks
            .iter()
            .any(|task| matches!(task.study, crate::workspace::StudyProgress::Running))
        {
            return;
        }
        let next = workspace.state.tasks.iter().find(|task| {
            study_launchable(task.state, &task.study, task.plan.study.enabled)
                && task.artifact.is_some()
        });
        let Some(task) = next.cloned() else {
            return;
        };
        let course = Course::from_completed(&crate::backend::Completed {
            out_dir: task.artifact.expect("filtered above"),
            title: task.plan.title,
            ..Default::default()
        });
        self.start_automatic_study(task.id, course, false, cx);
    }

    fn save_study_prompt_default(&mut self, cx: &mut Context<Self>) {
        if self.study_ui.phase != Phase::Ready {
            return;
        }
        let Some(input) = self.study_ui.prompt.as_ref() else {
            return;
        };
        let prompt = input.read(cx).value().to_string();
        if prompt.trim().is_empty() || prompt.len() > 131_072 {
            self.study_ui.error =
                Some(course2md::tr!("教学提示词不能为空，也不能超过 128 KiB。").into());
            cx.notify();
            return;
        }
        let mut next = self
            .preferences
            .generation_intent()
            .unwrap_or_else(|| self.preferences.generation())
            .clone();
        next.study_prompt = (prompt != self.study_ui.standard_prompt).then_some(prompt);
        match self.preferences.save_generation(next) {
            Ok(()) => {
                self.study_ui.error = None;
                self.study_ui.prompt_status = Some(
                    if self
                        .preferences
                        .is_pending(crate::preferences::PreferenceGroup::Generation)
                    {
                        course2md::tr!("默认教学提示词已提交保存。")
                    } else {
                        course2md::tr!("默认教学提示词已保存。")
                    }
                    .into(),
                );
            }
            Err(error) => {
                self.study_ui.error = Some(course2md::tr!("默认教学提示词保存失败：{}", error));
            }
        }
        cx.notify();
    }

    fn study_service(&self) -> Option<&ServiceVersion> {
        self.study_ui
            .service
            .as_deref()
            .and_then(|id| self.preferences.version(id))
            .filter(|version| {
                !self
                    .preferences
                    .service_retired_in_snapshot(&version.service_id)
            })
    }

    fn study_native_request(&self, cx: &App) -> anyhow::Result<NativeRequest> {
        let model = self
            .study_ui
            .model
            .as_ref()
            .map(|input| input.read(cx).value().to_string())
            .unwrap_or_default();
        Self::native_request_for_selection(
            &self.preferences,
            self.study_ui.service.as_deref(),
            &model,
        )
    }

    fn native_request_for_selection(
        preferences: &crate::preferences::Store,
        service: Option<&str>,
        model: &str,
    ) -> anyhow::Result<NativeRequest> {
        let id = service.ok_or_else(|| {
            anyhow::anyhow!(course2md::tr!("请选择本次 AI 服务，或改用导出到 ChatGPT。"))
        })?;
        let version = preferences.check_dispatch(id)?;
        anyhow::ensure!(
            version.config.protocol.purpose() == ServicePurpose::Ai,
            course2md::tr!("请选择本次 AI 服务，或改用导出到 ChatGPT。")
        );
        let model = model.trim();
        anyhow::ensure!(
            !model.is_empty() && model.len() <= 256 && !model.chars().any(char::is_control),
            course2md::tr!("请填写本次模型 ID，或从服务返回的模型中选择。")
        );
        if version.config.authentication == crate::preferences::Authentication::ApiKey {
            anyhow::ensure!(
                version.config.credential.is_some(),
                course2md::tr!("请先为此服务保存 API Key。")
            );
        }
        Ok(NativeRequest {
            settings: course2md::llm::LlmSettings {
                enabled: true,
                provider: version.config.protocol.llm_provider(),
                base_url: if version.config.protocol
                    == crate::preferences::ServiceProtocol::CodexResponses
                {
                    String::new()
                } else {
                    version.config.endpoint.clone()
                },
                model: model.to_owned(),
                vision: true,
                concurrency: 1,
                ..Default::default()
            },
            credential: version.config.credential.clone(),
            vault: preferences.vault(),
        })
    }

    fn fetch_study_models(&mut self, cx: &mut Context<Self>) {
        if self.study_ui.phase != Phase::Ready || self.study_ui.models.loading() {
            return;
        }
        let Some(version) = self.study_service().cloned() else {
            return;
        };
        let selected = version.id.clone();
        let revision = self.study_ui.revision;
        self.study_ui.catalog_generation += 1;
        let generation = self.study_ui.catalog_generation;
        if version.config.protocol == crate::preferences::ServiceProtocol::CodexResponses {
            self.study_ui.models.begin_external();
            let work =
                crate::spawn_blocking_io(course2md::login::codex::refresh_models_for_desktop);
            cx.spawn(async move |this, cx| {
                let result = work.recv().await;
                let _ = this.update(cx, |this, cx| {
                    if this.study_ui.revision != revision
                        || this.study_ui.catalog_generation != generation
                        || this.study_ui.service.as_ref() != Some(&selected)
                    {
                        return;
                    }
                    match result {
                        Ok(Ok(models)) => this
                            .study_ui
                            .models
                            .prime(models.into_iter().map(|(id, _)| id).collect()),
                        _ => this
                            .study_ui
                            .models
                            .reject(crate::model_discovery::Error::Custom(
                                course2md::tr!(
                                    "无法获取订阅模型列表，请检查账号；当前模型保持不变。"
                                )
                                .into(),
                            )),
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
                    self.study_ui.models.reject(error);
                    cx.notify();
                    return;
                }
            };
            let ticket = self.study_ui.models.begin(&request);
            let vault = self.preferences.vault();
            cx.spawn(async move |this, cx| {
                let result = crate::model_discovery::discover(request, vault).await;
                let _ = this.update(cx, |this, cx| {
                    if this.study_ui.revision != revision
                        || this.study_ui.catalog_generation != generation
                        || this.study_ui.service.as_ref() != Some(&selected)
                    {
                        return;
                    }
                    let current = this.study_service().and_then(|version| {
                        crate::model_discovery::RequestKey::from_draft(
                            &crate::preferences::ServiceDraft::from_version(version),
                            "",
                        )
                        .ok()
                    });
                    this.study_ui
                        .models
                        .complete(ticket, current.as_ref(), result);
                    cx.notify();
                });
            })
            .detach();
        }
        cx.notify();
    }

    pub(crate) fn study_running(&self) -> bool {
        self.study_ui.phase == Phase::Running
    }

    pub(crate) fn saved_study_output(&self, course: &Course) -> Option<PathBuf> {
        if self.study_ui.course == course.dir
            && self.study_ui.phase == Phase::Complete
            && self.study_ui.mode == Mode::Native
            && let Some(output) = &self.study_ui.output
        {
            return Some(output.clone());
        }
        self.workspace
            .as_ref()
            .and_then(|workspace| workspace.state.study_output_for(&course.dir))
            .map(Path::to_path_buf)
    }

    pub(crate) fn open_course_study(
        &mut self,
        course: Course,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(output) = self.saved_study_output(&course) {
            // A missing/moved file is reported by the IO worker. Never turn a
            // failed open into an implicit, potentially paid regeneration.
            self.open_study_document(output, cx);
        } else {
            self.open_study_dialog(course, window, cx);
        }
    }

    pub(crate) fn configure_study_regeneration(
        &mut self,
        course: Course,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.study_ui.phase.busy() {
            self.study_ui.phase = Phase::Idle;
        }
        self.open_study_dialog(course, window, cx);
    }

    pub(crate) fn open_study_dialog(
        &mut self,
        course: Course,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.study_ui.phase.busy()
            && (self.study_ui.course != course.dir
                || matches!(self.study_ui.phase, Phase::Idle | Phase::Failed))
        {
            let revision = self.study_ui.revision.wrapping_add(1);
            let same_course = self.study_ui.course == course.dir;
            let retrying = same_course && self.study_ui.phase == Phase::Failed;
            let prior_service = retrying.then(|| self.study_ui.service.clone()).flatten();
            let prior_model = retrying
                .then(|| {
                    self.study_ui
                        .model
                        .as_ref()
                        .map(|model| model.read(cx).value().to_string())
                })
                .flatten();
            let prior_prompt = retrying
                .then(|| {
                    self.study_ui
                        .prompt
                        .as_ref()
                        .map(|prompt| prompt.read(cx).value().to_string())
                })
                .flatten();
            let prior_mode = self.study_ui.mode;
            let prior_limit = self.study_ui.limit;
            let prior_show_prompt = self.study_ui.show_prompt;
            let output = same_course.then(|| self.study_ui.output.clone()).flatten();
            let selected = prior_service
                .as_deref()
                .or(self.preferences.default_refs().llm.as_deref())
                .and_then(|id| self.preferences.version(id))
                .cloned()
                .filter(|version| {
                    !self
                        .preferences
                        .service_retired_in_snapshot(&version.service_id)
                });
            let model = cx.new(|cx| InputState::new(window, cx));
            model.update(cx, |input, cx| {
                input.set_value(
                    prior_model.unwrap_or_else(|| {
                        selected
                            .as_ref()
                            .map(|version| version.config.model.clone())
                            .unwrap_or_default()
                    }),
                    window,
                    cx,
                )
            });
            let subscription = cx.subscribe(&model, |this, _, event: &InputEvent, cx| {
                if matches!(event, InputEvent::Change) {
                    this.study_ui.error = None;
                    cx.notify();
                }
            });
            let preferred_prompt =
                prior_prompt.or_else(|| self.preferences.generation().study_prompt.clone());
            self.study_ui = State {
                course: course.dir.clone(),
                title: course.title,
                phase: Phase::Inspecting,
                revision,
                output,
                service: selected.map(|version| version.id),
                model: Some(model),
                _model_subscription: Some(subscription),
                mode: if retrying { prior_mode } else { Mode::Native },
                limit: if retrying { prior_limit } else { Limit::Three },
                show_prompt: retrying && prior_show_prompt,
                ..Default::default()
            };
            let work = crate::spawn_blocking_io(move || inspect(&course.dir));
            cx.spawn_in(window, async move |this, cx| {
                let result = work.recv().await;
                let _ = this.update_in(cx, |this, window, cx| {
                    if this.study_ui.revision != revision {
                        return;
                    }
                    match result {
                        Ok(Ok((tool, description, standard_prompt))) => {
                            let prompt = cx.new(|cx| TextareaState::new(window, cx).rows(12));
                            prompt.update(cx, |input, cx| {
                                input.set_value(
                                    preferred_prompt.as_deref().unwrap_or(&standard_prompt),
                                    window,
                                    cx,
                                )
                            });
                            let subscription =
                                cx.subscribe(&prompt, |this, _, event: &InputEvent, cx| {
                                    if matches!(event, InputEvent::Change) {
                                        this.study_ui.prompt_status = None;
                                        this.study_ui.error = None;
                                        cx.notify();
                                    }
                                });
                            this.study_ui.tool = Some(tool);
                            this.study_ui.description = Some(description);
                            this.study_ui.standard_prompt = standard_prompt;
                            this.study_ui.prompt = Some(prompt);
                            this.study_ui._prompt_subscription = Some(subscription);
                            this.study_ui.phase = Phase::Ready;
                            if this.study_ui.mode == Mode::Native {
                                this.fetch_study_models(cx);
                            }
                        }
                        _ => {
                            this.study_ui.phase = Phase::Failed;
                            this.study_ui.error = Some(match result {
                                Ok(Err(error)) => error.to_string(),
                                _ => course2md::tr!("操作意外中断，请重试").into(),
                            });
                        }
                    }
                    cx.notify();
                });
            })
            .detach();
        }
        let desktop = cx.entity();
        let footer_desktop = desktop.clone();
        let content = cx.new(|cx| StudyDialog {
            _observation: cx.observe(&desktop, |_, _, cx| cx.notify()),
            desktop,
            footer: false,
        });
        let footer = cx.new(|cx| StudyDialog {
            _observation: cx.observe(&footer_desktop, |_, _, cx| cx.notify()),
            desktop: footer_desktop,
            footer: true,
        });
        window.open_dialog(cx, move |dialog: Dialog, window, _| {
            dialog
                .title(course2md::tr!("AI 学习讲义"))
                .w((window.bounds().size.width - px(32.)).min(px(640.)))
                .margin_top(task_dialog_top(window))
                .child(content.clone())
                .footer(footer.clone())
        });
        cx.notify();
    }

    fn refresh_study_description(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.study_ui.phase.busy() {
            return;
        }
        let revision = self.study_ui.revision.wrapping_add(1);
        self.study_ui.revision = revision;
        self.study_ui.phase = Phase::Inspecting;
        self.study_ui.error = None;
        self.study_ui.models.invalidate();
        self.study_ui.catalog_generation = self.study_ui.catalog_generation.wrapping_add(1);
        let course = self.study_ui.course.clone();
        let work = crate::spawn_blocking_io(move || inspect(&course));
        cx.spawn_in(window, async move |this, cx| {
            let result = work.recv().await;
            let _ = this.update_in(cx, |this, window, cx| {
                if this.study_ui.revision != revision {
                    return;
                }
                match result {
                    Ok(Ok((tool, description, standard_prompt))) => {
                        if let Some(prompt) = &this.study_ui.prompt {
                            if prompt.read(cx).value() == this.study_ui.standard_prompt {
                                prompt.update(cx, |input, cx| {
                                    input.set_value(&standard_prompt, window, cx)
                                });
                            }
                        } else {
                            let prompt = cx.new(|cx| TextareaState::new(window, cx).rows(12));
                            let selected = this
                                .preferences
                                .generation()
                                .study_prompt
                                .as_deref()
                                .unwrap_or(&standard_prompt);
                            prompt.update(cx, |input, cx| input.set_value(selected, window, cx));
                            let subscription =
                                cx.subscribe(&prompt, |this, _, event: &InputEvent, cx| {
                                    if matches!(event, InputEvent::Change) {
                                        this.study_ui.prompt_status = None;
                                        this.study_ui.error = None;
                                        cx.notify();
                                    }
                                });
                            this.study_ui.prompt = Some(prompt);
                            this.study_ui._prompt_subscription = Some(subscription);
                        }
                        this.study_ui.tool = Some(tool);
                        this.study_ui.description = Some(description);
                        this.study_ui.standard_prompt = standard_prompt;
                        this.study_ui.phase = Phase::Ready;
                        if this.study_ui.mode == Mode::Native {
                            this.fetch_study_models(cx);
                        }
                    }
                    _ => {
                        this.study_ui.phase = Phase::Failed;
                        this.study_ui.error = Some(match result {
                            Ok(Err(error)) => error.to_string(),
                            _ => course2md::tr!("操作意外中断，请重试").into(),
                        });
                    }
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    fn start_study_generation(&mut self, cx: &mut Context<Self>) {
        if !self.study_ui.phase.can_start() {
            return;
        }
        let Some(info) = self.study_ui.description.as_ref() else {
            return;
        };
        if info.unit_count == 0 {
            return;
        }
        let expected = info.clone();
        let Some(tool) = self.study_ui.tool.clone() else {
            return;
        };
        let course = self.study_ui.course.clone();
        let limit = self.study_ui.limit;
        let revision = self.study_ui.revision;
        let prompt = match self.study_ui.prompt.as_ref() {
            Some(input) => input.read(cx).value().to_string(),
            None => return,
        };
        if prompt.trim().is_empty() || prompt.len() > 131_072 {
            self.study_ui.error =
                Some(course2md::tr!("教学提示词不能为空，也不能超过 128 KiB。").into());
            cx.notify();
            return;
        }
        let override_prompt = (prompt != self.study_ui.standard_prompt).then_some(prompt);
        let native = if self.study_ui.mode == Mode::Native {
            let prepared = self.study_native_request(cx);
            match prepared {
                Ok(native) => Some(native),
                Err(error) => {
                    self.study_ui.error = Some(error.to_string());
                    cx.notify();
                    return;
                }
            }
        } else {
            None
        };
        self.study_ui.phase = Phase::Running;
        self.study_ui.meter = None;
        self.study_ui.output = None;
        self.study_ui.error = None;
        self.study_ui.export_zip = None;
        let work = study_worker(move |progress| {
            generate(
                tool,
                course,
                limit,
                expected,
                native,
                override_prompt,
                progress,
            )
        });
        cx.spawn(async move |this, cx| {
            let result = loop {
                match work.recv().await {
                    Ok(StudyEvent::Progress(meter)) => {
                        let _ = this.update(cx, |this, cx| {
                            if this.study_ui.revision == revision
                                && this.study_ui.phase == Phase::Running
                                && this
                                    .study_ui
                                    .meter
                                    .as_ref()
                                    .is_none_or(|previous| previous.accepts(&meter))
                            {
                                this.study_ui.meter = Some(meter);
                                cx.notify();
                            }
                        });
                    }
                    Ok(StudyEvent::Finished(result)) => break Ok(result),
                    Err(error) => break Err(error),
                }
            };
            let _ = this.update(cx, |this, cx| {
                if this.study_ui.revision != revision {
                    return;
                }
                match result {
                    Ok(Ok((phase, output, error, export_zip))) => {
                        this.study_ui.phase = phase;
                        this.study_ui.output = output;
                        this.study_ui.error = error;
                        this.study_ui.export_zip = export_zip;
                    }
                    _ => {
                        this.study_ui.phase = Phase::Failed;
                        this.study_ui.error = Some(match result {
                            Ok(Err(error)) => error.to_string(),
                            _ => course2md::tr!("操作意外中断，请重试").into(),
                        });
                    }
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    fn study_dialog_content(&self, cx: &mut Context<Self>) -> Div {
        let state = &self.study_ui;
        let mut body = v_flex()
            .w_full()
            .min_w_0()
            .gap_3()
            .child(
                div()
                    .whitespace_normal()
                    .font_weight(FontWeight::SEMIBOLD)
                    .child(state.title.clone()),
            )
            .child(help(course2md::tr!(
                "使用当前教学提示词，把对应截图和字幕整理为逐步讲解。"
            )));
        body = body.child(
            SingleChoiceGroup::new("study-mode", course2md::tr!("本次处理方式"))
                .segmented()
                .options([
                    ("native", course2md::tr!("自动生成")),
                    ("web", course2md::tr!("导出到 ChatGPT")),
                ])
                .selected(if state.mode == Mode::Native {
                    "native"
                } else {
                    "web"
                })
                .disabled(state.phase != Phase::Ready)
                .on_change(cx.listener(|this, id: &SharedString, _, cx| {
                    if this.study_ui.phase != Phase::Ready {
                        return;
                    }
                    this.study_ui.mode = if id.as_ref() == "web" {
                        Mode::ChatGptExport
                    } else {
                        Mode::Native
                    };
                    this.study_ui.models.invalidate();
                    this.study_ui.catalog_generation += 1;
                    this.study_ui.error = None;
                    if this.study_ui.mode == Mode::Native {
                        this.fetch_study_models(cx);
                    }
                    cx.notify();
                })),
        );
        if state.mode == Mode::Native {
            let selected = self.study_service();
            let selected_id = state.service.clone();
            let services: Vec<_> = self
                .preferences
                .latest_versions()
                .into_values()
                .filter(|version| {
                    version.config.protocol.purpose() == ServicePurpose::Ai
                        && !self
                            .preferences
                            .service_retired_in_snapshot(&version.service_id)
                })
                .collect();
            let weak = cx.weak_entity();
            body = body.child(outlined_field_label(
                "study-service-label",
                course2md::tr!("本次 AI 服务"),
                TEXT_BODY,
                control("study-service")
                    .w_full()
                    .label(
                        selected
                            .map(|version| version.config.name.clone())
                            .unwrap_or_else(|| course2md::tr!("请选择 AI 服务").into()),
                    )
                    .disabled(state.phase != Phase::Ready)
                    .dropdown_menu(move |menu, _, _| {
                        services
                            .iter()
                            .fold(menu.scrollable(true), |menu, version| {
                                let id = version.id.clone();
                                let model = version.config.model.clone();
                                let model_label = model.clone();
                                let name = version.config.name.clone();
                                let route =
                                    crate::settings_ui::service_form::ai_service_route_label(
                                        version.config.protocol,
                                    );
                                let weak = weak.clone();
                                menu.item(
                                    PopupMenuItem::element(move |_, _| {
                                        v_flex()
                                            .min_w_0()
                                            .max_w(px(420.))
                                            .gap_1()
                                            .child(div().whitespace_normal().child(name.clone()))
                                            .child(
                                                div()
                                                    .whitespace_normal()
                                                    .text_color(color(GRAY))
                                                    .child(format!("{} · {}", route, model_label)),
                                            )
                                    })
                                    .checked(selected_id.as_ref() == Some(&id))
                                    .on_click(
                                        move |_, window, cx| {
                                            let _ = weak.update(cx, |this, cx| {
                                                if this.study_ui.phase != Phase::Ready {
                                                    return;
                                                }
                                                this.study_ui.service = Some(id.clone());
                                                this.study_ui.models.invalidate();
                                                this.study_ui.catalog_generation += 1;
                                                this.study_ui.error = None;
                                                if let Some(input) = &this.study_ui.model {
                                                    input.update(cx, |input, cx| {
                                                        input.set_value(model.clone(), window, cx)
                                                    });
                                                }
                                                this.fetch_study_models(cx);
                                                cx.notify();
                                            });
                                        },
                                    ),
                                )
                            })
                    }),
            ));
            if let Some(selected) = selected {
                body = body.child(help(
                    crate::settings_ui::service_form::ai_service_billing_hint(
                        selected.config.protocol,
                    ),
                ));
            }
            if let Some(model) = &state.model {
                body = body
                    .child(outlined_field_label(
                        "study-model-label",
                        course2md::tr!("讲义模型"),
                        TEXT_BODY,
                        crate::model_discovery::model_field(
                            "study-model",
                            model,
                            &state.models,
                            state.phase != Phase::Ready,
                            None,
                            cx.listener(|this, _, _, cx| this.fetch_study_models(cx)),
                            cx,
                        ),
                    ))
                    .child(help(course2md::tr!(
                        "仅用于本次讲义；不会修改默认服务、其他任务或已生成结果。"
                    )));
            }
        } else {
            body = body.child(help(course2md::tr!(
                "只导出截图、字幕和本次教学提示词，不调用 API。请到 ChatGPT 官网上传材料并选择模型；网页版额度以官网提示为准。"
            )));
        }
        if let Some(prompt) = &state.prompt {
            let mut editor = v_flex()
                .w_full()
                .min_w_0()
                .gap_3()
                .child(
                    Textarea::new(prompt)
                        .h(px(240.))
                        .w_full()
                        .disabled(state.phase != Phase::Ready)
                        .aria_label(course2md::tr!("本次教学提示词")),
                )
                .child(help(course2md::tr!(
                    "修改只影响本次处理；保存为默认模板后，其他课程也会使用。编辑的是完整教学指令。"
                )))
                .child(
                    h_flex()
                        .gap_2()
                        .flex_wrap()
                        .child(
                            outline_pill("study-save-prompt")
                                .icon(icons::save())
                                .label(course2md::tr!("保存为默认模板"))
                                .disabled(state.phase != Phase::Ready)
                                .on_click(
                                    cx.listener(|this, _, _, cx| {
                                        this.save_study_prompt_default(cx)
                                    }),
                                ),
                        )
                        .child(
                            quiet("study-reset-prompt")
                                .label(course2md::tr!("恢复标准模板"))
                                .disabled(state.phase != Phase::Ready)
                                .on_click(cx.listener(|this, _, window, cx| {
                                    let standard = this.study_ui.standard_prompt.clone();
                                    if let Some(prompt) = &this.study_ui.prompt {
                                        prompt.update(cx, |input, cx| {
                                            input.set_value(standard, window, cx)
                                        });
                                    }
                                    this.study_ui.prompt_status = None;
                                    cx.notify();
                                })),
                        ),
                );
            if let Some(status) = &state.prompt_status {
                editor = editor.child(help(status.clone()));
            }
            body = body
                .child(
                    quiet("study-prompt-toggle")
                        .label(if state.show_prompt {
                            course2md::tr!("收起教学提示词")
                        } else {
                            course2md::tr!("编辑教学提示词")
                        })
                        .icon(if state.show_prompt {
                            icons::chevron_up()
                        } else {
                            icons::chevron_down()
                        })
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.study_ui.show_prompt = !this.study_ui.show_prompt;
                            cx.notify();
                        })),
                )
                .child(crate::motion::disclosure(
                    "study-prompt-editor",
                    state.show_prompt,
                    editor,
                ));
        }
        if let Some(info) = &state.description {
            body = body
                .child(help(course2md::tr!(
                    "可处理 {} 页 · 保存到 {}",
                    info.unit_count,
                    info.outputs_root.display()
                )))
                .child(
                    SingleChoiceGroup::new("study-page-limit", course2md::tr!("本次处理页数"))
                        .segmented()
                        .options([
                            ("3", course2md::tr!("3 页")),
                            ("10", course2md::tr!("10 页")),
                            ("all", course2md::tr!("全部")),
                        ])
                        .selected(state.limit.id())
                        .disabled(state.phase != Phase::Ready)
                        .on_change(cx.listener(|this, id: &SharedString, _, cx| {
                            if this.study_ui.phase == Phase::Ready
                                && let Some(limit) = Limit::from_id(id)
                            {
                                this.study_ui.limit = limit;
                                cx.notify();
                            }
                        })),
                );
            if info.unit_count == 0 {
                body = body.child(help(course2md::tr!(
                    "本课程没有可处理的截图与字幕，请先完成转录。"
                )));
            }
        }
        if state.phase == Phase::Inspecting {
            body = body.child(help(course2md::tr!(
                "正在读取本地课程和 AI 配置…不会发送 API 请求。"
            )));
        }
        if state.phase == Phase::Running {
            body = body.child(
                v_flex()
                    .gap_2()
                    .w_full()
                    .child(crate::study_progress::View {
                        id: "study-dialog-progress".into(),
                        meter: state.meter.clone(),
                        active: true,
                    })
                    .child(help(if state.mode == Mode::ChatGptExport {
                        course2md::tr!("正在整理上传材料…关闭窗口后仍会继续。")
                    } else {
                        course2md::tr!("正在生成讲义…关闭此窗口后仍会继续，可从阅读页再次查看。")
                    })),
            );
        }
        if state.phase == Phase::Complete {
            body = body.child(help(if state.mode == Mode::ChatGptExport {
                course2md::tr!("材料已导出，请在 ChatGPT 网页手动生成讲义。")
            } else {
                course2md::tr!("学习讲义已生成，可以打开阅读。")
            }));
        }
        if let Some((message, failed)) = &state.open_status {
            body = body.child(help(message.clone()).text_color(color(if *failed {
                DANGER
            } else {
                GRAY
            })));
        }
        if let Some(error) = &state.error {
            if state.phase != Phase::Running && state.meter.is_some() {
                body = body.child(crate::study_progress::View {
                    id: "study-dialog-retained".into(),
                    meter: state.meter.clone(),
                    active: false,
                });
            }
            body = body.child(
                div()
                    .whitespace_normal()
                    .text_color(color(DANGER))
                    .child(error.clone()),
            );
        }
        body
    }

    fn study_dialog_actions(&self, cx: &mut Context<Self>) -> Div {
        let state = &self.study_ui;
        let mut footer = v_flex().w_full().min_w_0().gap_3();
        let mut actions = h_flex().gap_2().flex_wrap().justify_end();
        if !state.phase.busy() && state.phase != Phase::Complete {
            let version = state.service.clone();
            actions = actions.child(
                quiet("study-manage-service")
                    .icon(icons::settings())
                    .label(course2md::tr!("管理服务与模型"))
                    .on_click(cx.listener(move |this, _, window, cx| {
                        window.close_dialog(cx);
                        this.open_settings_service_editor(
                            ServicePurpose::Ai,
                            version.clone(),
                            window,
                            cx,
                        );
                    })),
            );
        }
        if let Some(zip) = &state.export_zip {
            let zip = zip.clone();
            actions = actions.child(
                outline_pill("study-show-export")
                    .icon(icons::folder_open())
                    .label(course2md::tr!("查看上传材料包"))
                    .on_click(move |_, _, cx| cx.reveal_path(&zip)),
            );
        }
        if state.mode == Mode::ChatGptExport {
            actions = actions.child(
                quiet("study-chatgpt-website")
                    .label(course2md::tr!("打开 ChatGPT 官网"))
                    .on_click(|_, _, cx| cx.open_url("https://chatgpt.com")),
            );
        }
        if let Some(output) = &state.output {
            let reveal = output.clone();
            let output = output.clone();
            actions = actions.child(
                outline_pill("study-open-result")
                    .when(state.phase == Phase::Complete, |button| button.primary())
                    .icon(icons::book_open())
                    .label(if state.mode == Mode::ChatGptExport {
                        course2md::tr!("查看导出材料")
                    } else if state.phase == Phase::Complete {
                        course2md::tr!("打开学习讲义")
                    } else {
                        course2md::tr!("打开已生成部分")
                    })
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.open_study_document_with_feedback(
                            output.clone(),
                            Some(this.study_ui.revision),
                            cx,
                        )
                    })),
            );
            actions = actions.child(
                quiet("study-file-location")
                    .icon(icons::folder_open())
                    .label(course2md::tr!("查看讲义文件"))
                    .on_click(move |_, _, cx| cx.reveal_path(&reveal)),
            );
        }
        if state.phase.can_start() {
            let enabled = state
                .description
                .as_ref()
                .is_some_and(|info| info.unit_count > 0)
                && (state.mode == Mode::ChatGptExport
                    || (self.study_service().is_some()
                        && state
                            .model
                            .as_ref()
                            .is_some_and(|model| !model.read(cx).value().trim().is_empty())));
            footer = footer.child(help(if state.mode == Mode::ChatGptExport {
                course2md::tr!("导出只在本地整理材料；上传到官网需由你操作。")
            } else { course2md::tr!("开始后会把所选截图与字幕发送到本次服务；额度不足或失败时停止，不自动切换付费服务。") }));
            actions = actions.child(
                primary_pill("study-start")
                    .icon(icons::play_arrow())
                    .label(if state.mode == Mode::ChatGptExport {
                        course2md::tr!("导出上传材料")
                    } else {
                        course2md::tr!("开始生成")
                    })
                    .disabled(!enabled)
                    .on_click(cx.listener(|this, _, _, cx| this.start_study_generation(cx))),
            );
        }
        if matches!(state.phase, Phase::Complete | Phase::Failed) {
            actions = actions.child(
                quiet("study-prepare-again")
                    .label(course2md::tr!("调整设置后重新处理"))
                    .on_click(cx.listener(|this, _, window, cx| {
                        if this.study_ui.phase == Phase::Failed {
                            this.refresh_study_description(window, cx);
                        } else {
                            this.study_ui.phase = Phase::Ready;
                            this.study_ui.error = None;
                            cx.notify();
                        }
                    })),
            );
        }
        actions = actions.child(
            quiet("study-close")
                .label(course2md::tr!("关闭"))
                .on_click(|_, window, cx| window.close_dialog(cx)),
        );
        footer.child(actions)
    }
}

#[cfg(test)]
mod tests {
    use super::{
        Limit, Phase, request_args, result_phase, standard_prompt_from_script, study_launchable,
    };
    use crate::workspace::{StudyProgress, TaskState};
    use std::path::Path;
    #[test]
    fn teaching_launches_only_for_published_explicitly_enabled_work() {
        assert!(study_launchable(
            TaskState::Complete,
            &StudyProgress::Pending,
            true
        ));
        assert!(study_launchable(
            TaskState::Partial,
            &StudyProgress::Pending,
            true
        ));
        for state in [TaskState::Queued, TaskState::Running, TaskState::Uncertain] {
            assert!(!study_launchable(state, &StudyProgress::Pending, true));
        }
        assert!(!study_launchable(
            TaskState::Complete,
            &StudyProgress::Pending,
            false
        ));
        assert!(!study_launchable(
            TaskState::Complete,
            &StudyProgress::Running,
            true
        ));
        assert!(!study_launchable(
            TaskState::Complete,
            &StudyProgress::Complete {
                output: "done.html".into()
            },
            true,
        ));
    }
    #[test]
    fn standard_teaching_prompt_keeps_both_template_parts_in_order() {
        let temp = tempfile::tempdir().unwrap();
        std::fs::write(temp.path().join("task.txt"), "task instruction").unwrap();
        std::fs::write(temp.path().join("prompt43.txt"), "lesson instruction").unwrap();
        assert_eq!(
            standard_prompt_from_script(&temp.path().join("study.py")).unwrap(),
            "task instruction\nlesson instruction"
        );
    }
    #[test]
    fn busy_and_terminal_states_cannot_dispatch_another_paid_request() {
        for phase in [
            Phase::Idle,
            Phase::Inspecting,
            Phase::Running,
            Phase::Complete,
            Phase::Failed,
        ] {
            assert!(!phase.can_start());
        }
        assert!(Phase::Ready.can_start());
        assert!(Phase::Running.busy());
        assert!(Phase::Inspecting.busy());
        assert!(!Phase::Complete.busy());
    }
    #[test]
    fn command_keeps_course_and_result_paths_as_individual_arguments() {
        let script = Path::new("E:/项目 工具/study.py");
        let course = Path::new("E:/课程/复变 & 函数");
        let result = Path::new("E:/临时 结果/result.json");
        let args = request_args(script, course, Limit::Ten, result);
        assert_eq!(args[3], script.as_os_str());
        assert_eq!(args[5], course.as_os_str());
        assert_eq!(args[7], "10");
        assert_eq!(args[8], "--result-file");
        assert_eq!(args[9], result.as_os_str());
        assert_eq!(args.len(), 10);
        assert!(!args.iter().any(|arg| arg == "--open"));
        assert_eq!(Limit::default(), Limit::Three);
        assert_eq!(Limit::from_id("all"), Some(Limit::All));
        assert_eq!(Limit::from_id("--model=other"), None);
    }
    #[test]
    fn completion_requires_both_process_success_and_an_existing_output() {
        assert_eq!(result_phase("success", true, true), Phase::Complete);
        assert_eq!(result_phase("success", true, false), Phase::Failed);
        assert_eq!(result_phase("success", false, true), Phase::Failed);
        assert_eq!(result_phase("failed", true, true), Phase::Failed);
        assert_eq!(result_phase("unknown", true, true), Phase::Failed);
    }
}
