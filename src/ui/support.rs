use std::collections::HashSet;
use std::sync::Arc;

use gpui_kit::base::Selectable as _;
use gpui_kit::component::{
    ActiveTheme, Disableable, Icon, Sizable, WindowExt,
    button::{Button, ButtonVariants},
    h_flex,
    input::{InputEvent, Textarea, TextareaState},
    spinner::Spinner,
    v_flex,
};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use super::{IconName, widgets};
use crate::{
    backend::Backend,
    config::{self, ChatMessage},
    prefs,
};

const MAX_MESSAGE_LEN: usize = 4000;

pub enum SupportEvent {
    NewReplies(Vec<ChatMessage>),
}

pub struct SupportView {
    backend: Arc<Backend>,
    input: Entity<TextareaState>,
    messages: Vec<ChatMessage>,
    seen_ids: HashSet<String>,
    loaded_once: bool,
    loading: bool,
    sending: bool,
    attach_log: bool,
    /// Ids of messages sent with a log, for servers that don't report `has_log`.
    sent_with_log: HashSet<String>,
    error: Option<SharedString>,
    last_read_at: i64,
    is_open: bool,
    scroll: ScrollHandle,
    _subscription: Subscription,
}

impl EventEmitter<SupportEvent> for SupportView {}

impl SupportView {
    pub fn new(backend: Arc<Backend>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let input = cx.new(|cx| {
            TextareaState::new(window, cx)
                .auto_grow(3, 8)
                .placeholder("Опишите проблему или идею…")
        });
        let subscription =
            cx.subscribe_in(&input, window, |this, _, event: &InputEvent, window, cx| {
                if let InputEvent::PressEnter {
                    secondary: true, ..
                } = event
                {
                    this.send(window, cx);
                }
            });

        Self {
            backend,
            input,
            messages: Vec::new(),
            seen_ids: HashSet::new(),
            loaded_once: false,
            loading: false,
            sending: false,
            attach_log: false,
            sent_with_log: HashSet::new(),
            error: None,
            last_read_at: prefs::last_read_reply_at(),
            is_open: false,
            scroll: ScrollHandle::new(),
            _subscription: subscription,
        }
    }

    pub fn unread_count(&self) -> usize {
        self.messages
            .iter()
            .filter(|m| m.from_developer && m.created_at_ms > self.last_read_at)
            .count()
    }

    fn mark_read(&mut self) {
        let newest = self
            .messages
            .iter()
            .filter(|m| m.from_developer)
            .map(|m| m.created_at_ms)
            .max()
            .unwrap_or(0);
        if newest > self.last_read_at {
            self.last_read_at = newest;
            let _ = prefs::set_last_read_reply_at(newest);
        }
    }

    pub fn refresh(&mut self, cx: &mut Context<Self>) -> Task<()> {
        if !config::chat_available() || self.loading {
            return Task::ready(());
        }
        self.loading = true;
        cx.notify();

        let job = self.backend.spawn(config::list_developer_messages());
        cx.spawn(async move |this, cx| {
            let result = job.await.map_err(anyhow::Error::from).and_then(|r| r);
            let _ = this.update(cx, |this, cx| {
                this.loading = false;
                match result {
                    Ok(messages) => {
                        this.error = None;
                        this.apply_messages(messages, cx);
                    }
                    Err(e) => {
                        this.error = Some(format!("Не удалось загрузить переписку: {e}").into())
                    }
                }
                cx.notify();
            });
        })
    }

    fn apply_messages(&mut self, messages: Vec<ChatMessage>, cx: &mut Context<Self>) {
        let fresh: Vec<ChatMessage> = messages
            .iter()
            .filter(|m| m.from_developer && !self.seen_ids.contains(&m.id))
            .filter(|m| m.created_at_ms > self.last_read_at)
            .cloned()
            .collect();

        self.seen_ids = messages.iter().map(|m| m.id.clone()).collect();
        let grew = messages.len() != self.messages.len();
        self.messages = messages;

        if self.is_open {
            self.mark_read();
        } else if self.loaded_once && !fresh.is_empty() {
            cx.emit(SupportEvent::NewReplies(fresh));
        }
        self.loaded_once = true;
        if grew {
            self.scroll.scroll_to_bottom();
        }
    }

    fn send(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let text = self.input.read(cx).value().trim().to_string();
        if text.is_empty() || self.sending {
            return;
        }
        if text.chars().count() > MAX_MESSAGE_LEN {
            self.error = Some(
                format!("Сообщение слишком длинное (максимум {MAX_MESSAGE_LEN} символов)").into(),
            );
            cx.notify();
            return;
        }

        self.sending = true;
        self.error = None;
        cx.notify();

        let attach_log = self.attach_log;
        let job = self
            .backend
            .spawn(config::send_developer_message(text.clone(), attach_log));
        cx.spawn_in(window, async move |this, cx| {
            let result = job.await.map_err(anyhow::Error::from).and_then(|r| r);
            let _ = this.update_in(cx, |this, window, cx| {
                this.sending = false;
                match result {
                    Ok(saved) => {
                        this.input
                            .update(cx, |input, cx| input.set_value("", window, cx));
                        let mut saved = saved.unwrap_or_else(|| ChatMessage {
                            id: uuid::Uuid::new_v4().to_string(),
                            from_developer: false,
                            text,
                            created_at_ms: chrono::Utc::now().timestamp_millis(),
                            has_log: false,
                        });
                        if attach_log {
                            saved.has_log = true;
                            this.sent_with_log.insert(saved.id.clone());
                            this.attach_log = false;
                        }
                        this.seen_ids.insert(saved.id.clone());
                        this.messages.push(saved);
                        this.scroll.scroll_to_bottom();
                        this.refresh(cx).detach();
                    }
                    Err(e) => this.error = Some(format!("Не удалось отправить: {e}").into()),
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn set_open(&mut self, open: bool, cx: &mut Context<Self>) {
        self.is_open = open;
        if open {
            self.mark_read();
            self.scroll.scroll_to_bottom();
            self.refresh(cx).detach();
        }
        cx.notify();
    }

    fn set_attach_log(&mut self, value: bool, cx: &mut Context<Self>) {
        self.attach_log = value;
        cx.notify();
    }

    fn render_message(&self, message: &ChatMessage, cx: &App) -> impl IntoElement {
        let theme = cx.theme();
        let mine = !message.from_developer;
        let has_log = message.has_log || self.sent_with_log.contains(&message.id);
        h_flex().w_full().when(mine, |el| el.justify_end()).child(
            v_flex()
                .max_w(px(420.))
                .gap_1()
                .px_3()
                .py_2()
                .rounded(px(10.))
                .map(|el| {
                    if mine {
                        el.bg(theme.primary).text_color(theme.primary_foreground)
                    } else {
                        el.bg(theme.secondary)
                            .text_color(theme.secondary_foreground)
                    }
                })
                .child(div().text_xs().opacity(0.75).child(format!(
                    "{} · {}",
                    if mine {
                        "Вы"
                    } else {
                        "Разработчик"
                    },
                    format_time(message.created_at_ms)
                )))
                .child(div().text_sm().child(message.text.clone()))
                .when(has_log, |el| {
                    el.child(
                        h_flex()
                            .gap_1()
                            .text_xs()
                            .opacity(0.75)
                            .child(Icon::new(IconName::Paperclip).xsmall())
                            .child("Лог прикреплён"),
                    )
                }),
        )
    }
}

impl Render for SupportView {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();

        if !config::chat_available() {
            return v_flex()
                .gap_2()
                .child(widgets::hint(
                    "Связь с разработчиком недоступна в этой сборке приложения.",
                    cx,
                ))
                .into_any_element();
        }

        let empty = self.messages.is_empty();
        let can_send = !self.sending && !self.input.read(cx).value().trim().is_empty();

        v_flex()
            .gap_3()
            .child(widgets::hint(
                "Опишите проблему или идею. Ответ разработчика придёт сюда же — мы покажем уведомление, даже если окно закрыто.",
                cx,
            ))
            .child(
                div()
                    .id("support-messages")
                    .h(px(300.))
                    .p_3()
                    .rounded(px(8.))
                    .border_1()
                    .border_color(theme.border)
                    .bg(theme.background)
                    .overflow_y_scroll()
                    .track_scroll(&self.scroll)
                    .map(|el| {
                        if empty && self.loading {
                            el.child(
                                h_flex()
                                    .size_full()
                                    .justify_center()
                                    .gap_2()
                                    .child(Spinner::new().small())
                                    .child(widgets::hint("Загружаем переписку…", cx)),
                            )
                        } else if empty {
                            el.child(
                                v_flex()
                                    .size_full()
                                    .items_center()
                                    .justify_center()
                                    .gap_2()
                                    .text_color(theme.muted_foreground)
                                    .child(Icon::new(IconName::MessageCircle).large())
                                    .child(widgets::hint("Сообщений пока нет", cx)),
                            )
                        } else {
                            el.child(
                                v_flex()
                                    .gap_2()
                                    .children(self.messages.iter().map(|m| self.render_message(m, cx))),
                            )
                        }
                    }),
            )
            .when_some(self.error.clone(), |el, error| {
                el.child(div().text_sm().text_color(theme.danger).child(error))
            })
            .child(Textarea::new(&self.input).disabled(self.sending))
            .child(
                h_flex()
                    .gap_2()
                    .child(widgets::hint("Ctrl+Enter — отправить", cx).text_xs())
                    .child(div().flex_1())
                    .when(self.attach_log, |el| {
                        el.child(
                            h_flex()
                                .gap_1()
                                .pl_2()
                                .rounded(px(6.))
                                .bg(theme.secondary)
                                .text_xs()
                                .text_color(theme.secondary_foreground)
                                .child(Icon::new(IconName::Paperclip).xsmall())
                                .child("Лог будет прикреплён")
                                .child(
                                    Button::new("support-detach-log")
                                        .ghost()
                                        .xsmall()
                                        .icon(Icon::new(IconName::Close))
                                        .tooltip("Не прикреплять лог")
                                        .disabled(self.sending)
                                        .on_click(
                                            cx.listener(|this, _, _, cx| {
                                                this.set_attach_log(false, cx)
                                            }),
                                        ),
                                ),
                        )
                    })
                    .child(
                        Button::new("support-attach-log")
                            .ghost()
                            .small()
                            .icon(Icon::new(IconName::Paperclip))
                            .selected(self.attach_log)
                            .toggled(self.attach_log)
                            .tooltip(if self.attach_log {
                                "Не прикреплять лог"
                            } else {
                                "Прикрепить технический лог — он поможет разработчику найти причину проблемы"
                            })
                            .disabled(self.sending)
                            .on_click(cx.listener(|this, _, _, cx| {
                                let value = !this.attach_log;
                                this.set_attach_log(value, cx)
                            })),
                    )
                    .child(
                        Button::new("support-refresh")
                            .ghost()
                            .small()
                            .icon(Icon::new(IconName::RefreshCw))
                            .tooltip("Проверить ответы")
                            .loading(self.loading)
                            .on_click(cx.listener(|this, _, _, cx| this.refresh(cx).detach())),
                    )
                    .child(
                        Button::new("support-send")
                            .primary()
                            .small()
                            .icon(Icon::new(IconName::Send))
                            .label(if self.sending { "Отправка…" } else { "Отправить" })
                            .loading(self.sending)
                            .disabled(!can_send)
                            .on_click(cx.listener(|this, _, window, cx| this.send(window, cx))),
                    ),
            )
            .into_any_element()
    }
}

pub fn open(view: Entity<SupportView>, window: &mut Window, cx: &mut App) {
    view.update(cx, |view, cx| view.set_open(true, cx));
    let focus = view.read(cx).input.read(cx).focus_handle(cx);
    focus.focus(window, cx);

    window.open_dialog(cx, move |dialog, _, _| {
        let on_close = view.clone();
        dialog
            .title(
                h_flex()
                    .gap_2()
                    .child(Icon::new(IconName::LifeBuoy))
                    .child("Поддержка"),
            )
            .w(px(560.))
            .child(view.clone())
            .on_close(move |_, _, cx| {
                on_close.update(cx, |view, cx| view.set_open(false, cx));
            })
    });
}

fn format_time(ms: i64) -> String {
    chrono::DateTime::from_timestamp_millis(ms)
        .map(|t| {
            t.with_timezone(&chrono::Local)
                .format("%d.%m.%Y %H:%M")
                .to_string()
        })
        .unwrap_or_default()
}
