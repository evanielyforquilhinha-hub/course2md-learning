//! The service connection form shared by the settings editor and first-run setup:
//! draft, inputs, field errors, model discovery and key visibility. Each surface
//! lays the controls out in its own form; behavior and choices live here once.
use super::*;
use crate::codex_ui::CodexSurface;
use crate::model_discovery;
use crate::preferences::FieldError;

/// A route is distinct from the user's saved service name and exact model ID.
pub(crate) fn ai_service_route_label(protocol: ServiceProtocol) -> &'static str {
    match protocol {
        ServiceProtocol::CodexResponses => course2md::tr!("ChatGPT 订阅（Codex 通道）"),
        _ => protocol.choice_label(),
    }
}

pub(crate) fn ai_service_billing_hint(protocol: ServiceProtocol) -> &'static str {
    match protocol {
        ServiceProtocol::CodexResponses => course2md::tr!(
            "使用 ChatGPT 订阅的 Codex 通道，与 Work / Codex 共用额度；不使用普通 ChatGPT 对话额度。"
        ),
        ServiceProtocol::OllamaChat => course2md::tr!("使用本地 Ollama；不产生云端 API 费用。"),
        _ => course2md::tr!("API 服务由服务商单独计费；额度与价格以服务商控制台为准。"),
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum FormField {
    Name,
    Address,
    Model,
    Key,
}

impl FormField {
    /// The `FieldError::field` reported for this input.
    fn error_key(self) -> &'static str {
        match self {
            Self::Name => "name",
            Self::Address => "address",
            Self::Model => "model",
            Self::Key => "api_key",
        }
    }
    fn from_error_key(key: &str) -> Self {
        match key {
            "name" => Self::Name,
            "address" => Self::Address,
            "model" => Self::Model,
            _ => Self::Key,
        }
    }
}

/// Where a form lives. Asynchronous results land only on the form that asked.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum FormOwner {
    Editor,
    Setup(ServicePurpose),
}

impl FormOwner {
    fn codex_surface(self) -> CodexSurface {
        match self {
            Self::Editor => CodexSurface::Editor,
            Self::Setup(_) => CodexSurface::Onboarding,
        }
    }
}

pub(crate) struct ServiceForm {
    inputs: BTreeMap<FormField, Entity<InputState>>,
    pub(crate) draft: ServiceDraft,
    protocols: ProtocolDrafts,
    pub(crate) errors: Vec<FieldError>,
    pub(crate) models: model_discovery::State,
    pub(crate) show_key: bool,
    _subscriptions: Vec<Subscription>,
}

/// Unsaved values belong to their service type, including a key typed in this form.
/// A typed key is never published until Save and is wiped when this snapshot drops.
struct ProtocolSelection {
    draft: ServiceDraft,
    typed_key: zeroize::Zeroizing<String>,
}

#[derive(Default)]
struct ProtocolDrafts {
    inactive: BTreeMap<&'static str, ProtocolSelection>,
}

impl ProtocolDrafts {
    fn switch(
        &mut self,
        current: ServiceDraft,
        typed_key: zeroize::Zeroizing<String>,
        protocol: ServiceProtocol,
    ) -> ProtocolSelection {
        let old_address = current.address.trim().trim_end_matches('/');
        let old_host = preferences::service_host(old_address, current.protocol);
        self.inactive.insert(
            current.protocol.id(),
            ProtocolSelection {
                draft: current.clone(),
                typed_key,
            },
        );
        if let Some(saved) = self.inactive.remove(protocol.id()) {
            return saved;
        }
        let mut next = current;
        if next.name.trim().is_empty() || Some(next.name.trim()) == old_host.as_deref() {
            next.name.clear();
        }
        let ai_route_change = protocol.purpose() == ServicePurpose::Ai;
        next.protocol = protocol;
        if ai_route_change {
            next.model.clear();
            next.address = match protocol {
                ServiceProtocol::CodexResponses => preferences::CODEX_ADDRESS,
                ServiceProtocol::OllamaChat => preferences::OLLAMA_DEFAULT_ADDRESS,
                _ => "",
            }
            .into();
            next.authentication = if protocol.keyless() {
                Authentication::None
            } else {
                Authentication::ApiKey
            };
            next.credential = None;
            next.credential_source = None;
        } else if protocol.keyless() {
            next.authentication = Authentication::None;
        }
        ProtocolSelection {
            draft: next,
            typed_key: zeroize::Zeroizing::new(String::new()),
        }
    }
}

impl ServiceForm {
    pub(crate) fn new(
        owner: FormOwner,
        fields: &[FormField],
        draft: ServiceDraft,
        window: &mut Window,
        cx: &mut Context<Desktop>,
    ) -> Self {
        let inputs: BTreeMap<_, _> = fields
            .iter()
            .map(|&field| {
                let input = cx.new(|cx| {
                    InputState::new(window, cx)
                        .placeholder(if field == FormField::Address {
                            "https://api.example.com/v1"
                        } else {
                            ""
                        })
                        .masked(field == FormField::Key)
                });
                (field, input)
            })
            .collect();
        let subscriptions = inputs
            .iter()
            .map(|(&field, input)| {
                cx.subscribe_in(input, window, move |this, _, event, _, cx| {
                    if matches!(event, InputEvent::Change) {
                        this.service_form_input_changed(owner, field, cx);
                    }
                })
            })
            .collect();
        let mut form = Self {
            inputs,
            draft: ServiceDraft::new(draft.protocol.purpose()),
            protocols: ProtocolDrafts::default(),
            errors: Vec::new(),
            models: Default::default(),
            show_key: false,
            _subscriptions: subscriptions,
        };
        form.load(draft, window, cx);
        form
    }

    pub(crate) fn input(&self, field: FormField) -> &Entity<InputState> {
        &self.inputs[&field]
    }
    pub(crate) fn value(&self, field: FormField, cx: &App) -> String {
        self.inputs
            .get(&field)
            .map(|input| input.read(cx).value().to_string())
            .unwrap_or_default()
    }
    /// The key typed in this session; empty keeps the saved one.
    pub(crate) fn typed_key(&self, cx: &App) -> String {
        self.value(FormField::Key, cx)
    }
    /// The draft with what the inputs show now. Inputs set programmatically emit no
    /// change event, so the inputs, not the draft, are authoritative for text.
    pub(crate) fn current_draft(&self, cx: &App) -> ServiceDraft {
        let mut draft = self.draft.clone();
        if self.inputs.contains_key(&FormField::Name) {
            draft.name = self.value(FormField::Name, cx);
        }
        draft.address = self.value(FormField::Address, cx);
        draft.model = self.value(FormField::Model, cx);
        draft
    }

    /// Shows `draft` with an empty, masked key field.
    pub(crate) fn load(&mut self, draft: ServiceDraft, window: &mut Window, cx: &mut App) {
        self.protocols.inactive.clear();
        for (field, value) in [
            (FormField::Name, draft.name.clone()),
            (FormField::Address, draft.address.clone()),
            (FormField::Model, draft.model.clone()),
            (FormField::Key, String::new()),
        ] {
            if let Some(input) = self.inputs.get(&field) {
                input.update(cx, |input, cx| input.set_value(value, window, cx));
            }
        }
        let placeholder = if draft.credential.is_some() {
            "••••••••"
        } else {
            ""
        };
        self.inputs[&FormField::Key].update(cx, |input, cx| {
            input.set_placeholder(placeholder, window, cx);
            input.set_masked(true, window, cx);
        });
        self.draft = draft;
        self.errors.clear();
        self.models.invalidate();
        self.show_key = false;
    }
    pub(crate) fn clear_key(&self, window: &mut Window, cx: &mut App) {
        self.inputs[&FormField::Key].update(cx, |input, cx| input.set_value("", window, cx));
    }
    pub(crate) fn focus(&self, field: FormField, window: &mut Window, cx: &mut App) {
        if let Some(input) = self.inputs.get(&field) {
            input.update(cx, |input, cx| input.focus(window, cx));
        }
    }

    pub(crate) fn error(&self, field: FormField) -> Option<&str> {
        self.errors
            .iter()
            .find(|error| error.field == field.error_key())
            .map(|error| error.message.as_str())
    }
    /// Validates the staged draft and focuses the first invalid field.
    pub(crate) fn validate(&mut self, window: &mut Window, cx: &mut App) -> bool {
        self.errors = self.draft.validate();
        match self.errors.first() {
            Some(error) => {
                self.focus(FormField::from_error_key(error.field), window, cx);
                false
            }
            None => true,
        }
    }

    pub(crate) fn is_codex(&self) -> bool {
        self.draft.protocol == ServiceProtocol::CodexResponses
    }

    /// Each type keeps its own unsaved values while the form remains open.
    /// A first visit starts with that type's address and an empty model, rather
    /// than carrying a cloud model or endpoint into a different service.
    fn switch_protocol(&mut self, protocol: ServiceProtocol, window: &mut Window, cx: &mut App) {
        if self.draft.protocol == protocol {
            return;
        }
        let ProtocolSelection {
            draft: next,
            typed_key: key,
        } = self.protocols.switch(
            self.current_draft(cx),
            zeroize::Zeroizing::new(self.typed_key(cx)),
            protocol,
        );
        for (field, value) in [
            (FormField::Name, next.name.as_str()),
            (FormField::Address, next.address.as_str()),
            (FormField::Model, next.model.as_str()),
            (FormField::Key, key.as_str()),
        ] {
            if let Some(input) = self.inputs.get(&field) {
                input.update(cx, |input, cx| input.set_value(value, window, cx));
            }
        }
        self.inputs[&FormField::Key].update(cx, |input, cx| {
            input.set_placeholder(
                if next.credential.is_some() {
                    "••••••••"
                } else {
                    ""
                },
                window,
                cx,
            );
            input.set_masked(true, window, cx);
        });
        self.draft = next;
        self.errors.clear();
        self.models.invalidate();
        self.show_key = false;
    }
}

pub(crate) fn protocol_icon(protocol: ServiceProtocol) -> Icon {
    match protocol {
        ServiceProtocol::SpeechTranscriptions => icons::mic(),
        ServiceProtocol::SpeechChat => icons::article(),
        ServiceProtocol::AiChat => icons::cloud(),
        ServiceProtocol::OllamaChat => icons::server(),
        ServiceProtocol::CodexResponses => icons::sparkles(),
    }
}

impl Desktop {
    pub(crate) fn service_form(&self, owner: FormOwner) -> Option<&ServiceForm> {
        match owner {
            FormOwner::Editor => self.settings_ui.editor.as_ref().map(|editor| &editor.form),
            FormOwner::Setup(purpose) => self.setup_form(purpose),
        }
    }
    pub(crate) fn service_form_mut(&mut self, owner: FormOwner) -> Option<&mut ServiceForm> {
        match owner {
            FormOwner::Editor => self
                .settings_ui
                .editor
                .as_mut()
                .map(|editor| &mut editor.form),
            FormOwner::Setup(purpose) => self.setup_form_mut(purpose),
        }
    }

    /// An edit invalidates what the surface derived from the old values.
    /// A name change does not affect test results.
    fn service_form_edited(&mut self, owner: FormOwner, affects_tests: bool) {
        match owner {
            FormOwner::Editor => {
                if let Some(editor) = &mut self.settings_ui.editor {
                    editor.saved_configuration = None;
                    editor.status = None;
                    if affects_tests {
                        editor.evidence = None;
                    }
                }
            }
            FormOwner::Setup(purpose) => self.setup_service_edited(purpose),
        }
    }

    fn service_form_input_changed(
        &mut self,
        owner: FormOwner,
        field: FormField,
        cx: &mut Context<Self>,
    ) {
        let Some(form) = self.service_form_mut(owner) else {
            return;
        };
        form.errors.clear();
        if matches!(field, FormField::Address | FormField::Key) {
            form.models.invalidate();
        }
        self.service_form_edited(owner, field != FormField::Name);
        cx.notify();
    }

    fn switch_service_protocol(
        &mut self,
        owner: FormOwner,
        protocol: ServiceProtocol,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(form) = self.service_form_mut(owner) else {
            return;
        };
        form.switch_protocol(protocol, window, cx);
        self.service_form_edited(owner, true);
        if protocol == ServiceProtocol::CodexResponses {
            self.codex_refresh_status(owner.codex_surface(), cx);
        }
        cx.notify();
    }

    /// The service type choice for this form's purpose.
    pub(crate) fn service_protocol_choices(
        &self,
        owner: FormOwner,
        group: SingleChoiceGroup,
        cx: &mut Context<Self>,
    ) -> SingleChoiceGroup {
        let Some(form) = self.service_form(owner) else {
            return group;
        };
        let protocols: Vec<_> =
            ServiceProtocol::for_purpose(form.draft.protocol.purpose()).collect();
        protocols
            .iter()
            .fold(
                group.options(
                    protocols
                        .iter()
                        .map(|protocol| (protocol.id(), ai_service_route_label(*protocol))),
                ),
                |group, protocol| group.icon(protocol.id(), protocol_icon(*protocol)),
            )
            .selected(form.draft.protocol.id())
            .on_change(cx.listener(move |this, id: &SharedString, window, cx| {
                if let Some(protocol) = ServiceProtocol::from_id(id) {
                    this.switch_service_protocol(owner, protocol, window, cx);
                }
            }))
    }

    pub(crate) fn service_auth_choices(
        &self,
        owner: FormOwner,
        group: SingleChoiceGroup,
        cx: &mut Context<Self>,
    ) -> SingleChoiceGroup {
        let api_key = self
            .service_form(owner)
            .is_some_and(|form| form.draft.authentication == Authentication::ApiKey);
        group
            .options([("api_key", "API Key"), ("none", course2md::tr!("无需认证"))])
            .icon("api_key", icons::shield())
            .icon("none", icons::web())
            .selected(if api_key { "api_key" } else { "none" })
            .on_change(cx.listener(move |this, id: &SharedString, _, cx| {
                let Some(form) = this.service_form_mut(owner) else {
                    return;
                };
                form.draft.authentication = if id.as_ref() == "api_key" {
                    Authentication::ApiKey
                } else {
                    Authentication::None
                };
                form.errors.clear();
                form.models.invalidate();
                this.service_form_edited(owner, true);
                cx.notify();
            }))
    }

    /// The trailing 显示/隐藏 action of the key input.
    pub(crate) fn service_key_visibility(
        &self,
        owner: FormOwner,
        id: impl Into<ElementId>,
        cx: &mut Context<Self>,
    ) -> Button {
        let shown = self.service_form(owner).is_some_and(|form| form.show_key);
        quiet(id)
            .mr(-crate::theme::QUIET_INSET)
            .icon(if shown {
                icons::eye_off()
            } else {
                icons::eye()
            })
            .label(if shown {
                course2md::tr!("隐藏")
            } else {
                course2md::tr!("显示")
            })
            .tooltip(if shown {
                course2md::tr!("隐藏密钥")
            } else {
                course2md::tr!("显示密钥")
            })
            .on_click(cx.listener(move |this, _, window, cx| {
                if let Some(form) = this.service_form_mut(owner) {
                    form.show_key = !form.show_key;
                    let masked = !form.show_key;
                    form.inputs[&FormField::Key]
                        .update(cx, |input, cx| input.set_masked(masked, window, cx));
                }
                cx.notify();
            }))
    }

    /// The model input with its discovery menu and field error.
    pub(crate) fn service_model_field(
        &self,
        owner: FormOwner,
        id: &'static str,
        disabled: bool,
        cx: &mut Context<Self>,
    ) -> Div {
        let Some(form) = self.service_form(owner) else {
            return div();
        };
        model_discovery::model_field(
            id,
            form.input(FormField::Model),
            &form.models,
            disabled,
            form.error(FormField::Model),
            cx.listener(move |this, _, _, cx| this.fetch_service_models(owner, cx)),
            cx,
        )
    }

    fn fetch_service_models(&mut self, owner: FormOwner, cx: &mut Context<Self>) {
        let Some(form) = self.service_form(owner) else {
            return;
        };
        if form.is_codex() {
            self.codex_refresh_models(owner.codex_surface(), cx);
            return;
        }
        let request = model_discovery::Request::from_draft(
            &form.current_draft(cx),
            Secret::new(form.typed_key(cx)),
        );
        let vault = self.preferences.vault();
        let Some(form) = self.service_form_mut(owner) else {
            return;
        };
        let request = match request {
            Ok(request) => request,
            Err(error) => {
                form.models.reject(error);
                cx.notify();
                return;
            }
        };
        let ticket = form.models.begin(&request);
        cx.spawn(async move |this, cx| {
            let result = model_discovery::discover(request, vault).await;
            let _ = this.update(cx, |this, cx| {
                let current = this.service_form(owner).and_then(|form| {
                    model_discovery::RequestKey::from_draft(
                        &form.current_draft(cx),
                        &form.typed_key(cx),
                    )
                    .ok()
                });
                // A reloaded or closed form has dropped this ticket.
                if let Some(form) = this.service_form_mut(owner) {
                    form.models.complete(ticket, current.as_ref(), result);
                    cx.notify();
                }
            });
        })
        .detach();
        cx.notify();
    }

    /// The form a Codex result belongs to, while it still edits a Codex service.
    fn codex_form(&mut self, surface: CodexSurface) -> Option<&mut ServiceForm> {
        let owner = match surface {
            CodexSurface::Editor => FormOwner::Editor,
            CodexSurface::Onboarding => FormOwner::Setup(ServicePurpose::Ai),
            CodexSurface::Settings => return None,
        };
        self.service_form_mut(owner).filter(|form| form.is_codex())
    }
    pub(crate) fn codex_form_needs_catalog(&mut self, surface: CodexSurface) -> bool {
        self.codex_form(surface)
            .is_some_and(|form| form.models.models().is_empty() && !form.models.loading())
    }
    pub(crate) fn codex_form_models_loading(&mut self, surface: CodexSurface) -> bool {
        self.codex_form(surface)
            .map(|form| form.models.begin_external())
            .is_some()
    }
    pub(crate) fn codex_form_models_failed(&mut self, surface: CodexSurface, message: String) {
        if let Some(form) = self.codex_form(surface) {
            form.models.reject(model_discovery::Error::Custom(message));
        }
    }
    /// Candidates only: a suggestion fills an empty model input and changes nothing saved.
    pub(crate) fn codex_form_catalog(
        &mut self,
        surface: CodexSurface,
        models: Vec<String>,
        suggested: Option<String>,
        cx: &mut Context<Self>,
    ) {
        let window = self.window;
        if let Some(form) = self.codex_form(surface) {
            form.models.prime(models);
            let input = form.input(FormField::Model).clone();
            crate::codex_ui::autofill_input(window, input, suggested, cx);
        }
    }
    pub(crate) fn codex_forms_logged_out(&mut self) {
        for surface in [CodexSurface::Editor, CodexSurface::Onboarding] {
            if let Some(form) = self.codex_form(surface) {
                form.models.invalidate();
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{Authentication, ProtocolDrafts, ServiceDraft, ServiceProtocol, ServicePurpose};
    use zeroize::Zeroizing;

    #[test]
    fn switching_to_local_and_back_keeps_the_compatible_model() {
        let mut draft = ServiceDraft::new(ServicePurpose::Ai);
        draft.address = "https://dashscope.aliyuncs.com/compatible-mode/v1/chat/completions".into();
        draft.model = "qwen-plus".into();
        draft.authentication = Authentication::ApiKey;
        let mut protocols = ProtocolDrafts::default();
        let local = protocols.switch(
            draft.clone(),
            Zeroizing::new("temporary-key".to_owned()),
            ServiceProtocol::OllamaChat,
        );
        assert_eq!(local.draft.address, "http://localhost:11434");
        assert_eq!(local.draft.model, "");
        assert_eq!(local.draft.authentication, Authentication::None);
        assert_eq!(local.typed_key.as_str(), "");
        let restored = protocols.switch(
            local.draft,
            Zeroizing::new(String::new()),
            ServiceProtocol::AiChat,
        );
        assert_eq!(restored.draft.model, "qwen-plus");
        assert_eq!(restored.draft.address, draft.address);
        assert_eq!(restored.draft.authentication, Authentication::ApiKey);
        assert_eq!(restored.typed_key.as_str(), "temporary-key");
        let codex = protocols.switch(
            restored.draft,
            restored.typed_key,
            ServiceProtocol::CodexResponses,
        );
        assert_eq!(codex.draft.model, "");
        assert_eq!(codex.draft.address, super::preferences::CODEX_ADDRESS);
        let again = protocols.switch(
            codex.draft,
            Zeroizing::new(String::new()),
            ServiceProtocol::AiChat,
        );
        assert_eq!(again.draft.model, "qwen-plus");
        assert_eq!(again.typed_key.as_str(), "temporary-key");
    }
}
