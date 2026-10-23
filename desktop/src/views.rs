//! Native pages: stable navigation and actions surround independently scrolling content.
use super::*;
use crate::theme::*;
use gpui_component::button::*;

const SHELL_GUTTER: f32 = 24.;
const SETTINGS_SHELL_WIDTH: f32 = 1108.;

/// The app bar marks the current destination with a tonal pill; TitleBar's
/// default bottom hairline would be unused chrome.
pub(crate) fn apply_shell_title_bar_chrome(bar: TitleBar) -> TitleBar {
    bar.pl_0().bg(color(CANVAS)).border_b_0()
}

fn sidebar_is_compact(window: &Window) -> bool {
    let scale = f32::from(window.rem_size()) / 14.;
    f32::from(window.bounds().size.width) < 720. * scale
}
fn sidebar_width(window: &Window) -> f32 {
    let scale = f32::from(window.rem_size()) / 14.;
    if sidebar_is_compact(window) {
        64. * scale
    } else {
        (if f32::from(window.bounds().size.width) < 1000. * scale {
            164.
        } else {
            196.
        }) * scale
    }
}
fn content_pane_width(window: &Window) -> f32 {
    (f32::from(window.bounds().size.width) - sidebar_width(window)).max(0.)
}

fn shell_column_for(page: Page) -> Div {
    shell_column_at(shell_column_width(page))
}

fn shell_column_width(page: Page) -> AbsoluteLength {
    match page {
        // Each task gets a useful content width: a gallery needs room for covers.
        Page::Settings => rems(SETTINGS_SHELL_WIDTH / 14.).into(),
        Page::Library => rems(1168. / 14.).into(),
        Page::New => rems(1200. / 14.).into(),
        _ => COLUMN.into(),
    }
}

/// The max width includes both gutters; layout calculations use the same box.
pub(super) fn shell_content_width(page: Page, window: &Window) -> f32 {
    let width = content_pane_width(window);
    let gutter = if matches!(page, Page::New | Page::Settings) && is_compact(window) {
        16.
    } else {
        SHELL_GUTTER
    };
    (width.min(f32::from(
        shell_column_width(page).to_pixels(window.rem_size()),
    )) - gutter * 2.)
        .max(0.)
}

/// Settings use top-level tabs within the content pane, without a second sidebar.
pub(super) fn settings_content_width(window: &Window) -> f32 {
    shell_content_width(Page::Settings, window)
}

/// Entrance motion replays when the notice text changes, not on every frame.
fn notice_key(text: &str) -> usize {
    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    text.hash(&mut hasher);
    hasher.finish() as usize
}

fn shell_column_at(width: AbsoluteLength) -> Div {
    div()
        .w_full()
        .min_w_0()
        .max_w(width)
        .mx_auto()
        .px(px(SHELL_GUTTER))
}

impl Desktop {
    /// Native window chrome stays small; destinations remain in the persistent sidebar.
    fn shell_topbar(&self, _window: &mut Window, _cx: &mut Context<Self>) -> TitleBar {
        apply_shell_title_bar_chrome(TitleBar::new().h(px(28.)))
    }

    fn shell_sidebar(&self, window: &mut Window, cx: &mut Context<Self>) -> Stateful<Div> {
        let compact = sidebar_is_compact(window);
        let navigation_scroll = window
            .use_keyed_state("primary-navigation-scroll", cx, |_, _| ScrollHandle::new())
            .read(cx)
            .clone();
        let settings_selected = self.page == Page::Settings;
        let item = |id: &'static str, label: &'static str, icon: Icon, selected: bool| {
            sidebar_action(id, label, icon, selected, compact, cx)
        };
        let courses = item(
            "sidebar-courses",
            course2md::tr!("全部课程"),
            icons::book_open(),
            matches!(self.page, Page::Library | Page::Result),
        )
        .on_click(cx.listener(|this, _, _, cx| {
            this.folder_filter = FolderFilter::All;
            this.navigate(Page::Library, cx);
        }));
        let add = item(
            "sidebar-add",
            course2md::tr!("添加课程"),
            icons::plus(),
            self.page == Page::New,
        )
        .on_click(cx.listener(|this, _, window, cx| {
            if this.prepare_workbench_input(window, cx) {
                this.generation_options_open = true;
                this.navigate(Page::New, cx);
            }
        }));
        let task_count = self.workspace.as_ref().map_or(0, |workspace| {
            crate::task_ui::actionable_task_count(&workspace.state.tasks)
        });
        let task_hint = self
            .workspace
            .as_ref()
            .and_then(|workspace| {
                workspace.state.tasks.iter().find(|task| {
                    matches!(
                        task.state,
                        workspace::TaskState::NeedsAttention
                            | workspace::TaskState::Uncertain
                            | workspace::TaskState::Partial
                    )
                })
            })
            .map(crate::task_ui::task_attention_summary);
        let tasks = item(
            "sidebar-tasks",
            course2md::tr!("任务"),
            if task_hint.is_some() {
                icons::warning()
            } else {
                icons::task()
            },
            self.page == Page::Task,
        )
        .when(!compact && task_count > 0, |button| {
            button.child(div().text_size(TEXT_AUX).child(task_count.to_string()))
        })
        .when_some(task_hint, |button, hint| button.tooltip(hint))
        .on_click(cx.listener(|this, _, _, cx| this.navigate(Page::Task, cx)));
        let settings = item(
            "sidebar-settings",
            course2md::tr!("设置"),
            if self.settings_have_problem() {
                icons::warning()
            } else {
                icons::settings()
            },
            settings_selected,
        )
        .on_click(cx.listener(|this, _, window, cx| {
            this.open_settings(window, cx);
        }));
        v_flex()
            .id("primary-sidebar")
            .w(px(sidebar_width(window)))
            .h_full()
            .min_h_0()
            .flex_shrink_0()
            .p(px(12.))
            .gap_2()
            .bg(color(INSET))
            .border_r_1()
            .border_color(color(HAIRLINE))
            .child(
                h_flex()
                    .min_h(px(48.))
                    .flex_shrink_0()
                    .items_center()
                    .px_2()
                    .when(compact, |view| view.child(icons::book_open().size_6()))
                    .when(!compact, |view| {
                        view.child(
                            div()
                                .text_size(TEXT_TITLE)
                                .font_weight(FontWeight::SEMIBOLD)
                                .child("course2md"),
                        )
                    }),
            )
            .child(
                div()
                    .relative()
                    .w_full()
                    .flex_1()
                    .min_h_0()
                    .child(
                        v_flex()
                            .id("primary-navigation")
                            .w_full()
                            .h_full()
                            .min_h_0()
                            .gap_2()
                            .overflow_y_scroll()
                            .track_scroll(&navigation_scroll)
                            .child(crate::focus_scroll::RevealFocus::new(
                                "reveal-sidebar-courses",
                                courses,
                                navigation_scroll.clone(),
                            ))
                            .child(crate::focus_scroll::RevealFocus::new(
                                "reveal-sidebar-add",
                                add,
                                navigation_scroll.clone(),
                            ))
                            .child(crate::focus_scroll::RevealFocus::new(
                                "reveal-sidebar-tasks",
                                tasks,
                                navigation_scroll.clone(),
                            )),
                    )
                    .child(crate::backend::vertical_scrollbar(&navigation_scroll)),
            )
            .child(settings)
    }

    fn task_result_notice(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        // A new result can briefly announce itself. Historical unresolved work
        // remains in Tasks; it must not consume every unrelated page's height.
        let (announced_id, shown) = self.transient_task_result.as_ref()?;
        if shown.elapsed() >= std::time::Duration::from_secs(8) {
            return None;
        }
        let visible_task = if self.page == Page::New {
            self.current_input_task(cx).map(|task| task.id.as_str())
        } else {
            None
        };
        let task = self
            .workspace
            .as_ref()?
            .state
            .tasks
            .iter()
            .filter(|task| {
                &task.id == announced_id
                    && task.unread
                    && task.handled_by.is_none()
                    && visible_task != Some(task.id.as_str())
                    && !(self.reading
                        && self.page == Page::New
                        && self.following_conversion.as_ref().is_some_and(|follow| {
                            follow.follows(&task.id, self.preview_generation)
                        }))
            })
            .max_by_key(|task| task.updated)?;
        let id = task.id.clone();
        let dismiss_id = id.clone();
        let exports_only = task.exports_only() && task.state == workspace::TaskState::Complete;
        let result = if matches!(
            task.state,
            workspace::TaskState::Complete | workspace::TaskState::Partial
        ) {
            task.artifact.clone().map(|out_dir| Completed {
                out_dir,
                title: task.plan.title.clone(),
                ..Default::default()
            })
        } else {
            None
        };
        let action = match task.state {
            workspace::TaskState::Complete if exports_only => course2md::tr!("打开导出位置"),
            workspace::TaskState::Complete => course2md::tr!("阅读笔记"),
            workspace::TaskState::Partial if result.is_some() => course2md::tr!("阅读笔记"),
            workspace::TaskState::Partial => course2md::tr!("查看未完成部分"),
            workspace::TaskState::Uncertain => course2md::tr!("查看结果"),
            _ => course2md::tr!("查看任务"),
        };
        Some(crate::motion::enter(
            SharedString::from(format!("task-result-notice-{}", task.id)),
            shell_column_for(self.page).py_2().flex_shrink_0().child(
                notice_banner(
                    "background-task-result",
                    NoticeKind::Info,
                    course2md::tr!(
                        "《{}》：{}",
                        task.plan.title,
                        if exports_only {
                            course2md::tr!("文件已导出")
                        } else {
                            task.state.label()
                        },
                    ),
                )
                .child(
                    control("open-task-result")
                        .icon(icons::arrow_forward())
                        .label(action)
                        .on_click(cx.listener(move |this, _, _, cx| {
                            if exports_only {
                                this.open_task_export_location(id.clone(), cx);
                            } else if let Some(done) = &result {
                                this.open_course(Course::from_completed(done), cx);
                            } else {
                                this.select_task(&id, cx);
                                this.navigate(Page::Task, cx);
                            }
                        })),
                )
                .child(
                    control("dismiss-task-result")
                        .ghost()
                        .icon(crate::icons::close())
                        .accessibility_label(course2md::tr!("关闭提示"))
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.transient_task_result = None;
                            if let Some(workspace) = &mut this.workspace
                                && let Err(error) = workspace.transaction(|state| {
                                    if let Some(task) = state.task_mut(&dismiss_id) {
                                        task.unread = false;
                                    }
                                    Ok(())
                                })
                            {
                                this.workspace_error = Some(course2md::tr!(
                                    "提示关闭状态未能保存：{}",
                                    format!("{error:#}")
                                ));
                            }
                            cx.notify();
                        })),
                ),
            ),
        ))
    }
    fn page_title(&self) -> String {
        match self.page {
            Page::New => course2md::tr!("生成笔记").into(),
            Page::Library => course2md::tr!("全部课程").into(),
            Page::Task => course2md::tr!("任务").into(),
            Page::Settings => course2md::tr!("设置").into(),
            Page::Result => self
                .preview
                .as_ref()
                .map(|p| p.course.title.clone())
                .unwrap_or_else(|| course2md::tr!("笔记").into()),
        }
    }
    fn page_header(&self, window: &Window, cx: &mut Context<Self>) -> Div {
        let row = h_flex()
            .w_full()
            .min_w_0()
            .gap_3()
            .flex_wrap()
            .h_auto()
            .min_h(rems(3.5))
            .items_center()
            .justify_between()
            .child(
                theme::page_heading("page-title", self.page_title())
                    .flex_1()
                    .min_w_0(),
            )
            .when(self.library_controls_visible(cx), |row| {
                let available = shell_content_width(Page::Library, window);
                let scale = f32::from(window.rem_size()) / 14.;
                row.child(
                    self.library_search(window, cx)
                        .w(px(if available < 600. * scale {
                            available
                        } else {
                            360. * scale
                        })),
                )
                .child(
                    primary_pill("library-new-note")
                        .icon(icons::plus())
                        .label(course2md::tr!("添加课程"))
                        .on_click(cx.listener(|this, _, window, cx| {
                            if this.prepare_workbench_input(window, cx) {
                                this.navigate(Page::New, cx);
                            }
                        })),
                )
            });
        v_flex().pt(px(24.)).pb(px(16.)).child(row)
    }
}

impl Render for Desktop {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        theme::apply_preference(&self.preferences.application().appearance, window, cx);
        theme::apply_scale(self.preferences.application().font_scale, window, cx);
        if self.onboarding.active {
            let content = self.onboarding_page(window, cx);
            return div()
                .id("onboarding-action-root")
                .track_focus(&self.root_focus)
                .tab_stop(false)
                .size_full()
                .child(a11y::ModalBackground::new(
                    v_flex()
                        .size_full()
                        .bg(color(CANVAS))
                        .text_color(color(INK))
                        .text_size(TEXT_BODY)
                        .when(!cfg!(target_os = "windows"), |view| {
                            view.child(
                                TitleBar::new()
                                    .h(title_bar_height(window))
                                    .pl_0()
                                    .bg(color(CANVAS))
                                    .border_color(color(HAIRLINE))
                                    .child(
                                        h_flex()
                                            .w_full()
                                            .justify_center()
                                            .gap_2()
                                            .items_center()
                                            .child(
                                                icons::book_open().size_4().text_color(color(GRAY)),
                                            )
                                            .child("course2md"),
                                    ),
                            )
                        })
                        .child(content),
                    window.has_active_dialog(cx),
                ))
                .children(Root::render_dialog_layer(window, cx))
                .into_any_element();
        }
        let content = match self.page {
            Page::New => self.new_page(window, cx),
            Page::Task => self.queue_page(window, cx),
            Page::Library => self.library_page(window, cx),
            Page::Settings => self.settings_page(window, cx),
            Page::Result => self.reader_page(window, cx),
        };
        // The workbench owns its source and options panes. Never put both
        // beneath a shared page scroll that can carry the source input away.
        let self_scrolling = matches!(self.page, Page::New | Page::Task | Page::Library);
        let content = v_flex()
            .gap_4()
            .when(
                matches!(self.page, Page::Result | Page::Settings) || self_scrolling,
                |v| v.h_full().min_h_0(),
            )
            .when(
                // The workbench card already shows a followed result opening.
                self.reading
                    && !matches!(self.page, Page::Library | Page::Result)
                    && !(self.page == Page::New && self.following_conversion.is_some()),
                |v| {
                    v.child(
                        h_flex()
                            .items_center()
                            .gap(px(8.))
                            .child(crate::motion::spinner("opening-note-spinner", cx))
                            .child(
                                accessible_text("opening-note", course2md::tr!("正在打开笔记…"))
                                    .role(Role::Status)
                                    .text_color(color(GRAY)),
                            ),
                    )
                },
            )
            .child(content);
        let topbar = self.shell_topbar(window, cx);
        let sidebar = self.shell_sidebar(window, cx);
        let task_notice = self.task_result_notice(cx);
        let model_notice = self.onboarding_background_notice(cx);
        let body = v_flex()
            .flex_1()
            .h_full()
            .min_w_0()
            .min_h_0()
            .w_full()
            .when_some(task_notice, |body, notice| body.child(notice))
            .when_some(model_notice, |body, notice| {
                body.child(shell_column_for(self.page).py_2().child(notice))
            })
            .when(self.page == Page::Library, |v| {
                v.child(
                    shell_column_for(self.page)
                        .flex_shrink_0()
                        .child(self.page_header(window, cx)),
                )
            })
            .when(self.page == Page::Library, |v| {
                v.child(shell_column_for(self.page).child(self.library_toolbar(window, cx)))
            })
            .when(
                self.page == Page::Library
                    && self
                        .settings_group_notice(crate::preferences::PreferenceGroup::Application)
                        .is_some_and(|(_, error)| error),
                |v| {
                    v.child(
                        shell_column_for(self.page).pb_3().flex_shrink_0().child(
                            div()
                                .id("library-settings-feedback")
                                .w_full()
                                .min_w_0()
                                .p_3()
                                .rounded(RADIUS_CARD)
                                .bg(color(DANGER_BG))
                                .child(self.group_feedback(
                                    crate::preferences::PreferenceGroup::Application,
                                    cx,
                                )),
                        ),
                    )
                },
            )
            .when_some(self.workspace_error.clone(), |v, message| {
                v.child(crate::motion::enter(
                    ("workspace-error", notice_key(&message)),
                    shell_column_for(self.page).pb_3().child(
                        notice_banner("workspace-error", NoticeKind::Danger, message)
                            .child(
                                control("retry-workspace-records")
                                    .icon(icons::refresh())
                                    .label(course2md::tr!("重试"))
                                    .disabled(self.job.is_some() || self.storage_ui.busy)
                                    .on_click(cx.listener(|this, _, window, cx| {
                                        this.retry_workspace_records(window, cx)
                                    })),
                            )
                            .when(self.workspace.is_none(), |row| {
                                row.child(
                                    control("rebuild-workspace-records")
                                        .icon(icons::restart())
                                        .label(course2md::tr!("备份并重建任务记录"))
                                        .disabled(self.job.is_some() || self.storage_ui.busy)
                                        .on_click(cx.listener(|this, _, window, cx| {
                                            this.rebuild_workspace_records(window, cx)
                                        })),
                                )
                            }),
                    ),
                ))
            })
            .when_some(self.message.clone(), |v, message| {
                v.child(crate::motion::state_enter(
                    ("app-message", notice_key(&message)),
                    shell_column_for(self.page).pb_3().child(
                        notice_banner("app-message", NoticeKind::Info, message).child(
                            control("dismiss-message")
                                .ghost()
                                .icon(crate::icons::close())
                                .accessibility_label(course2md::tr!("关闭提示"))
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.message = None;
                                    cx.notify();
                                })),
                        ),
                    ),
                ))
            })
            .child(
                div()
                    .id(("page-scroll", self.page as usize))
                    .flex_1()
                    .min_h_0()
                    .min_w_0()
                    .w_full()
                    .when(
                        !matches!(self.page, Page::Result | Page::Settings) && !self_scrolling,
                        |view| {
                            view.overflow_y_scroll()
                                .track_scroll(&self.scrolls[self.page as usize])
                        },
                    )
                    .child(
                        shell_column_for(self.page)
                            .when(self.page == Page::Settings, |v| v.max_w_full())
                            .when(
                                matches!(self.page, Page::New | Page::Settings)
                                    && is_compact(window),
                                |v| v.px(px(16.)),
                            )
                            .when(
                                matches!(self.page, Page::Result | Page::Settings)
                                    || self_scrolling,
                                |v| v.h_full().min_h_0(),
                            )
                            .when(self.page != Page::Settings && !self_scrolling, |v| v.pb_6())
                            .child(content),
                    ),
            );
        let background = v_flex()
            .size_full()
            .bg(color(CANVAS))
            .text_color(color(INK))
            .text_size(rems(1.))
            .when(!cfg!(target_os = "windows"), |view| view.child(topbar))
            .child(
                h_flex()
                    .flex_1()
                    .min_h_0()
                    .w_full()
                    .items_stretch()
                    .child(sidebar)
                    .child(body),
            );
        div()
            .id("desktop-action-root")
            .relative()
            .group("workbench-drop")
            .track_focus(&self.root_focus)
            .tab_stop(false)
            .size_full()
            .on_action(cx.listener(|this, _: &ImportVideo, window, cx| {
                if !window.has_active_dialog(cx) {
                    this.import_video_from_action(window, cx);
                }
            }))
            .on_action(cx.listener(|this, _: &SearchContent, window, cx| {
                if !window.has_active_dialog(cx) {
                    this.search_content(window, cx);
                }
            }))
            .on_action(cx.listener(|this, _: &OpenSettings, window, cx| {
                if !window.has_active_dialog(cx) {
                    this.open_settings(window, cx);
                }
            }))
            .on_action(cx.listener(|this, _: &OpenAbout, window, cx| {
                if !window.has_active_dialog(cx) {
                    this.open_about(window, cx);
                }
            }))
            .child(a11y::ModalBackground::new(
                background,
                window.has_active_dialog(cx),
            ))
            .when(self.page == Page::New, |view| {
                view.child(self.workbench_drop_overlay(cx))
            })
            .children(Root::render_dialog_layer(window, cx))
            .into_any_element()
    }
}
