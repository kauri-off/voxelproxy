use std::rc::Rc;
use std::time::Duration;

use gpui_kit::component::{
    ActiveTheme, Disableable, Icon, StyledExt, WindowExt,
    button::{Button, ButtonVariants},
    h_flex,
    text::TextView,
    v_flex,
};
use gpui_kit::*;

use super::{IconName, widgets};
use crate::{backend, changelog::ChangelogEntry};

const MANUAL_WARNING_SECONDS: u32 = 10;

type WindowCallback = Rc<dyn Fn(&mut Window, &mut App)>;

fn dialog_title(icon: IconName, color: Hsla, text: impl Into<SharedString>) -> impl IntoElement {
    h_flex()
        .gap_2()
        .child(Icon::new(icon).text_color(color))
        .child(text.into())
}

pub fn open_changelog(entries: Vec<ChangelogEntry>, window: &mut Window, cx: &mut App) {
    let newest = entries
        .first()
        .map(|e| e.version.clone())
        .unwrap_or_default();
    let oldest = entries
        .last()
        .map(|e| e.version.clone())
        .unwrap_or_default();
    let title: SharedString = if entries.len() > 1 {
        format!("Обновление {oldest} → {newest}").into()
    } else {
        format!("Что нового в {newest}").into()
    };
    let entries = Rc::new(entries);

    window.open_dialog(cx, move |dialog, _, cx| {
        let theme = cx.theme();
        dialog
            .title(dialog_title(
                IconName::Sparkles,
                theme.primary,
                title.clone(),
            ))
            .w(px(600.))
            .child(
                div()
                    .id("changelog-body")
                    .max_h(px(420.))
                    .overflow_y_scroll()
                    .child(v_flex().gap_4().children(entries.iter().map(|entry| {
                        v_flex()
                            .gap_1()
                            .child(
                                div()
                                    .text_sm()
                                    .font_semibold()
                                    .text_color(theme.primary)
                                    .child(format!("v{}", entry.version)),
                            )
                            .child(TextView::markdown(
                                SharedString::from(format!("changelog-{}", entry.version)),
                                entry.markdown.clone(),
                            ))
                    }))),
            )
            .footer(
                h_flex().w_full().justify_end().child(
                    Button::new("changelog-ok")
                        .primary()
                        .label("Понятно")
                        .on_click(|_, window, cx| window.close_dialog(cx)),
                ),
            )
    });
}

struct ManualWarningView {
    remaining: u32,
    on_accept: WindowCallback,
    _countdown: Task<()>,
}

impl ManualWarningView {
    fn new(on_accept: WindowCallback, cx: &mut Context<Self>) -> Self {
        let countdown = cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor().timer(Duration::from_secs(1)).await;
                let done = this.update(cx, |this, cx| {
                    this.remaining = this.remaining.saturating_sub(1);
                    cx.notify();
                    this.remaining == 0
                });
                if !matches!(done, Ok(false)) {
                    break;
                }
            }
        });
        Self {
            remaining: MANUAL_WARNING_SECONDS,
            on_accept,
            _countdown: countdown,
        }
    }
}

impl Render for ManualWarningView {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let ready = self.remaining == 0;
        let on_accept = self.on_accept.clone();
        let strong =
            |text: &'static str, color: Hsla| div().font_semibold().text_color(color).child(text);

        v_flex()
            .gap_3()
            .text_sm()
            .child(
                h_flex()
                    .flex_wrap()
                    .gap_1()
                    .child("Ручной режим")
                    .child(strong("палится", theme.danger))
                    .child("на проверке: когда модератор смотрит ваш ПК через AnyDesk или Discord, он видит, что игра подключена к локальному адресу, а не к серверу, и за это")
                    .child(strong("банят.", theme.danger)),
            )
            .child(div().child(
                "Он оставлен только для отладки и тестирования. Для обычной игры выберите «Через раздачу Wi-Fi» — он не палится.",
            ))
            .child(
                div()
                    .p_3()
                    .rounded(px(8.))
                    .bg(theme.danger.opacity(0.1))
                    .child("Продолжая, вы подтверждаете, что понимаете риски и берёте ответственность за возможный бан на себя."),
            )
            .child(
                h_flex()
                    .pt_2()
                    .gap_2()
                    .justify_end()
                    .child(
                        Button::new("manual-cancel")
                            .ghost()
                            .label("Отмена")
                            .on_click(|_, window, cx| window.close_dialog(cx)),
                    )
                    .child(
                        Button::new("manual-accept")
                            .danger()
                            .disabled(!ready)
                            .label(if ready {
                                "Понимаю риски, запустить".to_string()
                            } else {
                                format!("Прочитайте предупреждение… {}", self.remaining)
                            })
                            .on_click(move |_, window, cx| on_accept(window, cx)),
                    ),
            )
    }
}

pub fn open_manual_warning(
    on_accept: impl Fn(&mut Window, &mut App) + 'static,
    window: &mut Window,
    cx: &mut App,
) {
    let on_accept: WindowCallback = Rc::new(on_accept);
    let view = cx.new(|cx| ManualWarningView::new(on_accept, cx));
    window.open_dialog(cx, move |dialog, _, cx| {
        dialog
            .title(dialog_title(
                IconName::TriangleAlert,
                cx.theme().danger,
                "Опасный режим",
            ))
            .w(px(520.))
            .child(view.clone())
    });
}

// ShellExecuteW pumps messages while UAC is shown, so it must not run inside a GPUI callback.
pub fn relaunch_as_admin(on_error: impl FnOnce(String, &mut App) + 'static, cx: &mut App) {
    let job = cx
        .background_executor()
        .spawn(async { backend::relaunch_as_admin() });
    cx.spawn(async move |cx| match job.await {
        Ok(()) => cx.update(|cx| cx.quit()),
        Err(e) => cx.update(|cx| on_error(e, cx)),
    })
    .detach();
}

pub fn open_admin_required(
    on_error: impl Fn(String, &mut App) + 'static,
    window: &mut Window,
    cx: &mut App,
) {
    let on_error = Rc::new(on_error);
    window.open_dialog(cx, move |dialog, _, cx| {
        let on_error = on_error.clone();
        dialog
            .title(dialog_title(
                IconName::ShieldAlert,
                cx.theme().warning,
                "Нужны права администратора",
            ))
            .w(px(520.))
            .child(
                v_flex()
                    .gap_3()
                    .text_sm()
                    .child("Чтобы читать трафик раздачи Wi-Fi, VoxelProxy должен работать от имени администратора. Сейчас он запущен без этих прав.")
                    .child(widgets::hint(
                        "Нажмите кнопку ниже и подтвердите запрос Windows. Приложение перезапустится — затем снова нажмите «Запустить».",
                        cx,
                    )),
            )
            .footer(
                h_flex()
                    .w_full()
                    .gap_2()
                    .justify_end()
                    .child(
                        Button::new("admin-cancel")
                            .ghost()
                            .label("Отмена")
                            .on_click(|_, window, cx| window.close_dialog(cx)),
                    )
                    .child(
                        Button::new("admin-relaunch")
                            .primary()
                            .icon(Icon::new(IconName::ShieldCheck))
                            .label("Перезапустить от администратора")
                            .on_click(move |_, window, cx| {
                                window.close_dialog(cx);
                                let on_error = on_error.clone();
                                relaunch_as_admin(move |e, cx| on_error(e, cx), cx);
                            }),
                    ),
            )
    });
}
